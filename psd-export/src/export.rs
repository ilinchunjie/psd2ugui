use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use image::{GrayImage, ImageBuffer, RgbaImage};
use sha1::{Digest, Sha1};

use crate::effects::bake_layer_effects;
use crate::error::{AppError, Result};
use crate::manifest::{
    AssetRef, Bounds, DocumentInfo, ExportManifest, ExportWarning, LayerNode, LayerType, MaskRef,
    SourceInfo,
};
use crate::photoshop::{
    self, PhotoshopExportOptions, PhotoshopExportRequest, PhotoshopExportResponse,
    PhotoshopLayerExportRequest, PhotoshopLayerExportResult,
};
use crate::psd::{self, FlatLayer, MaskBitmap, ParseOptions, RgbaBitmap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterBackend {
    RawPsd,
    Photoshop,
    Auto,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub out_dir: PathBuf,
    pub include_hidden: bool,
    pub with_preview: bool,
    pub strict: bool,
    pub raster_backend: RasterBackend,
    pub photoshop_exe: Option<PathBuf>,
    pub photoshop_timeout_sec: u64,
}

#[derive(Debug, Clone)]
struct BuildNode {
    raw_index: usize,
    name: String,
    asset_file_stem: String,
    layer_type: LayerType,
    visible: bool,
    opacity: f32,
    blend_mode: String,
    bounds: Bounds,
    children: Vec<BuildNode>,
    image: Option<RgbaBitmap>,
    mask: Option<MaskBitmap>,
    text: Option<crate::manifest::TextInfo>,
    effects: crate::manifest::LayerEffects,
    unsupported: Vec<crate::manifest::UnsupportedInfo>,
    is_clipped: bool,
    id: String,
    stack_index: u32,
    clip_to: Option<String>,
    path_indices: Vec<usize>,
    external_asset: Option<StagedAsset>,
}

#[derive(Debug, Clone)]
struct StagedAsset {
    staged_path: PathBuf,
    relative_path: String,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
struct PreviewOverride {
    staged_path: PathBuf,
    asset: AssetRef,
}

pub fn export_psd_file(input: &Path, options: ExportOptions) -> Result<ExportManifest> {
    let parsed = psd::parse_psd(
        input,
        ParseOptions {
            strict: options.strict,
            with_preview: options.with_preview,
        },
    )?;

    let mut warnings = parsed.warnings.clone();
    let mut nodes = build_layer_tree(parsed.layers, &mut warnings)?;
    assign_ids_and_stack_indices(&mut nodes, &mut Vec::new());
    assign_clip_targets(&mut nodes, &mut warnings);
    assign_asset_file_stems(&mut nodes);

    let preview_override = if matches!(options.raster_backend, RasterBackend::RawPsd) {
        None
    } else {
        rasterize_with_photoshop(input, &options, &mut nodes, &mut warnings)?
    };

    bake_node_effects(&mut nodes, &mut warnings);

    if options.strict && !warnings.is_empty() {
        return Err(AppError::StrictWarnings {
            warning_count: warnings.len(),
        });
    }

    prepare_output_dirs(&options.out_dir)?;

    let preview_asset = if let Some(preview) = preview_override {
        copy_staged_asset(
            &preview.staged_path,
            &options.out_dir.join("preview").join("document.png"),
        )?;
        preview.asset
    } else {
        let preview = parsed.preview.as_ref().ok_or_else(|| {
            AppError::Manifest(
                "preview bitmap is missing; export cannot produce preview/document.png".to_string(),
            )
        })?;
        write_preview(&options.out_dir, preview)?
    };

    let layers = write_nodes(&options.out_dir, &mut nodes, options.include_hidden)?;

    let manifest = ExportManifest {
        schema_version: "2.1.0".to_string(),
        source: SourceInfo {
            input_path: parsed.source.input_path,
            input_file: parsed.source.input_file,
            file_size: parsed.source.file_size,
            file_sha1: parsed.source.file_sha1,
        },
        document: DocumentInfo {
            width: parsed.metadata.width,
            height: parsed.metadata.height,
            color_mode: parsed.metadata.color_mode,
            depth: parsed.metadata.depth,
            channel_count: parsed.metadata.channel_count,
            preview: preview_asset,
        },
        warnings,
        layers,
    };

    let manifest_path = options.out_dir.join("manifest.json");
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(&manifest_path, manifest_json)
        .map_err(|error| AppError::io(&manifest_path, error))?;

    Ok(manifest)
}

fn rasterize_with_photoshop(
    input: &Path,
    options: &ExportOptions,
    nodes: &mut [BuildNode],
    warnings: &mut Vec<ExportWarning>,
) -> Result<Option<PreviewOverride>> {
    let staging_dir = options.out_dir.join(".photoshop-stage");
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(|error| AppError::io(&staging_dir, error))?;
    }
    fs::create_dir_all(&staging_dir).map_err(|error| AppError::io(&staging_dir, error))?;

    let requests = collect_photoshop_layer_requests(nodes, options.include_hidden);
    let absolute_input = fs::canonicalize(input).map_err(|error| AppError::io(input, error))?;
    let request = PhotoshopExportRequest {
        source_psd_path: photoshop_path_string(&absolute_input),
        staging_dir: staging_dir.to_string_lossy().replace('\\', "/"),
        preview_relpath: "preview/document.png".to_string(),
        layers: requests.clone(),
    };

    let response = match photoshop::run_photoshop_export(
        &request,
        &PhotoshopExportOptions {
            photoshop_exe: options.photoshop_exe.clone(),
            timeout_sec: options.photoshop_timeout_sec,
        },
        &options.out_dir,
    ) {
        Ok(response) => response,
        Err(error) => {
            if matches!(options.raster_backend, RasterBackend::Auto) {
                warnings.push(ExportWarning::new(
                    "photoshop-export-fallback",
                    format!("Photoshop raster export failed and fell back to rawpsd: {error}"),
                ));
                return Ok(None);
            }
            return Err(error);
        }
    };

    apply_photoshop_response(
        nodes,
        &requests,
        &response,
        &staging_dir,
        options.raster_backend,
        warnings,
    )
}

fn build_layer_tree(
    layers: Vec<FlatLayer>,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Vec<BuildNode>> {
    match reconstruct_bottom_up(layers.clone()) {
        Ok(nodes) => Ok(nodes),
        Err(primary_error) => match reconstruct_bottom_up(layers.into_iter().rev().collect()) {
            Ok(nodes) => {
                warnings.push(ExportWarning::new(
                    "layer-order-fallback",
                    format!(
                        "used reversed raw layer order while rebuilding groups: {primary_error}"
                    ),
                ));
                Ok(nodes)
            }
            Err(secondary_error) => Err(AppError::Manifest(format!(
                "failed to rebuild PSD layer hierarchy: {primary_error}; reversed fallback also failed: {secondary_error}"
            ))),
        },
    }
}

fn reconstruct_bottom_up(layers: Vec<FlatLayer>) -> std::result::Result<Vec<BuildNode>, String> {
    let mut stack: Vec<Vec<BuildNode>> = vec![Vec::new()];

    for layer in layers {
        if layer.group_closer {
            stack.push(Vec::new());
            continue;
        }

        if layer.group_opener {
            if stack.len() == 1 {
                return Err(format!(
                    "group opener '{}' had no matching closer",
                    layer.name
                ));
            }

            let children = stack
                .pop()
                .ok_or_else(|| "group stack underflow".to_string())?;
            let mut node = BuildNode::from_flat_layer(layer);
            node.children = children.into_iter().rev().collect();
            stack
                .last_mut()
                .ok_or_else(|| "group parent stack missing".to_string())?
                .push(node);
            continue;
        }

        stack
            .last_mut()
            .ok_or_else(|| "layer stack missing".to_string())?
            .push(BuildNode::from_flat_layer(layer));
    }

    if stack.len() != 1 {
        return Err("unbalanced group markers remained after reconstruction".to_string());
    }

    Ok(stack.pop().unwrap().into_iter().rev().collect())
}

fn assign_ids_and_stack_indices(nodes: &mut [BuildNode], path: &mut Vec<usize>) {
    let sibling_count = nodes.len();
    for (visual_index, node) in nodes.iter_mut().enumerate() {
        path.push(visual_index);
        node.stack_index = (sibling_count - 1 - visual_index) as u32;
        node.id = generate_node_id(path, node.raw_index);
        node.path_indices = path.clone();
        assign_ids_and_stack_indices(&mut node.children, path);
        path.pop();
    }
}

fn assign_clip_targets(nodes: &mut [BuildNode], warnings: &mut Vec<ExportWarning>) {
    let mut clip_base: Option<String> = None;
    for index in (0..nodes.len()).rev() {
        if nodes[index].is_clipped {
            if let Some(base) = clip_base.clone() {
                nodes[index].clip_to = Some(base);
            } else {
                warnings.push(ExportWarning::for_layer(
                    "clipping-base-missing",
                    "clipped layer did not have a non-clipped sibling beneath it",
                    nodes[index].name.clone(),
                ));
            }
        } else {
            clip_base = Some(nodes[index].id.clone());
        }

        assign_clip_targets(&mut nodes[index].children, warnings);
    }
}

fn assign_asset_file_stems(nodes: &mut [BuildNode]) {
    let mut used_stems = HashMap::new();
    assign_asset_file_stems_recursive(nodes, &mut used_stems);
}

fn assign_asset_file_stems_recursive(
    nodes: &mut [BuildNode],
    used_stems: &mut HashMap<String, usize>,
) {
    for node in nodes {
        node.asset_file_stem = next_asset_file_stem(&node.name, used_stems);
        assign_asset_file_stems_recursive(&mut node.children, used_stems);
    }
}

fn next_asset_file_stem(name: &str, used_stems: &mut HashMap<String, usize>) -> String {
    let sanitized = sanitize_file_stem(name);
    let collision_key = sanitized.to_lowercase();
    let next_index = used_stems
        .entry(collision_key)
        .and_modify(|count| *count += 1)
        .or_insert(1);

    if *next_index == 1 {
        sanitized
    } else {
        format!("{sanitized} ({next_index})")
    }
}

fn collect_photoshop_layer_requests(
    nodes: &[BuildNode],
    include_hidden: bool,
) -> Vec<PhotoshopLayerExportRequest> {
    let mut requests = Vec::new();
    collect_photoshop_layer_requests_recursive(nodes, include_hidden, &mut requests);
    requests
}

fn collect_photoshop_layer_requests_recursive(
    nodes: &[BuildNode],
    include_hidden: bool,
    requests: &mut Vec<PhotoshopLayerExportRequest>,
) {
    for node in nodes {
        if should_request_photoshop_raster(node, include_hidden) {
            requests.push(PhotoshopLayerExportRequest {
                id: node.id.clone(),
                name: node.name.clone(),
                raw_index: node.raw_index,
                path_indices: node.path_indices.clone(),
                expected_visible: node.visible,
                output_png_relpath: asset_relative_path(node),
            });
        }
        collect_photoshop_layer_requests_recursive(&node.children, include_hidden, requests);
    }
}

fn should_request_photoshop_raster(node: &BuildNode, include_hidden: bool) -> bool {
    matches!(node.layer_type, LayerType::Pixel | LayerType::Shape)
        && node.bounds.width > 0
        && node.bounds.height > 0
        && (include_hidden || node.visible)
}

fn can_write_image_asset(node: &BuildNode) -> bool {
    (node.image.is_some() || node.external_asset.is_some())
        && !matches!(
            node.layer_type,
            LayerType::Group | LayerType::Text | LayerType::Unknown
        )
}

fn apply_photoshop_response(
    nodes: &mut [BuildNode],
    requests: &[PhotoshopLayerExportRequest],
    response: &PhotoshopExportResponse,
    staging_dir: &Path,
    backend: RasterBackend,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Option<PreviewOverride>> {
    let request_map = requests
        .iter()
        .map(|request| (request.id.clone(), request))
        .collect::<HashMap<_, _>>();
    let result_map = response
        .layers
        .iter()
        .map(|result| (result.id.clone(), result))
        .collect::<HashMap<_, _>>();

    for warning in &response.warnings {
        warnings.push(ExportWarning::new(
            "photoshop-export-warning",
            format!("Photoshop export warning: {warning}"),
        ));
    }

    apply_photoshop_response_recursive(
        nodes,
        &request_map,
        &result_map,
        staging_dir,
        backend,
        warnings,
    )?;

    preview_override_from_response(response, staging_dir, backend, warnings)
}

fn apply_photoshop_response_recursive(
    nodes: &mut [BuildNode],
    request_map: &HashMap<String, &PhotoshopLayerExportRequest>,
    result_map: &HashMap<String, &PhotoshopLayerExportResult>,
    staging_dir: &Path,
    backend: RasterBackend,
    warnings: &mut Vec<ExportWarning>,
) -> Result<()> {
    for node in nodes {
        if let Some(request) = request_map.get(&node.id) {
            let result = result_map.get(&node.id).copied();
            apply_photoshop_result_to_node(node, request, result, staging_dir, backend, warnings)?;
        }
        apply_photoshop_response_recursive(
            &mut node.children,
            request_map,
            result_map,
            staging_dir,
            backend,
            warnings,
        )?;
    }
    Ok(())
}

fn apply_photoshop_result_to_node(
    node: &mut BuildNode,
    request: &PhotoshopLayerExportRequest,
    result: Option<&PhotoshopLayerExportResult>,
    staging_dir: &Path,
    backend: RasterBackend,
    warnings: &mut Vec<ExportWarning>,
) -> Result<()> {
    let Some(result) = result else {
        return handle_photoshop_layer_failure(
            node,
            backend,
            warnings,
            "Photoshop response did not include this layer".to_string(),
        );
    };

    for warning in &result.warnings {
        warnings.push(ExportWarning::for_layer(
            "photoshop-layer-warning",
            format!("Photoshop layer export warning: {warning}"),
            node.name.clone(),
        ));
    }

    if !result.exported {
        return handle_photoshop_layer_failure(
            node,
            backend,
            warnings,
            "Photoshop did not export a PNG for this layer".to_string(),
        );
    }

    let Some(bounds) = result.bounds.clone() else {
        return handle_photoshop_layer_failure(
            node,
            backend,
            warnings,
            "Photoshop export did not return layer bounds".to_string(),
        );
    };
    let Some(width) = result.width else {
        return handle_photoshop_layer_failure(
            node,
            backend,
            warnings,
            "Photoshop export did not return layer width".to_string(),
        );
    };
    let Some(height) = result.height else {
        return handle_photoshop_layer_failure(
            node,
            backend,
            warnings,
            "Photoshop export did not return layer height".to_string(),
        );
    };

    let staged_path = staging_dir.join(request.output_png_relpath.replace('/', "\\"));
    if !staged_path.exists() {
        return handle_photoshop_layer_failure(
            node,
            backend,
            warnings,
            format!(
                "Photoshop reported success but the staged PNG is missing: {}",
                staged_path.display()
            ),
        );
    }

    node.bounds = bounds;
    node.external_asset = Some(StagedAsset {
        staged_path,
        relative_path: request.output_png_relpath.clone(),
        width,
        height,
    });
    node.effects.baked = vec!["photoshop_raster".to_string()];

    Ok(())
}

fn handle_photoshop_layer_failure(
    node: &BuildNode,
    backend: RasterBackend,
    warnings: &mut Vec<ExportWarning>,
    reason: String,
) -> Result<()> {
    if matches!(backend, RasterBackend::Photoshop) {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: {reason}",
            node.name
        )));
    }

    warnings.push(ExportWarning::for_layer(
        "photoshop-layer-fallback",
        format!("fell back to rawpsd raster export because {reason}"),
        node.name.clone(),
    ));
    Ok(())
}

fn preview_override_from_response(
    response: &PhotoshopExportResponse,
    staging_dir: &Path,
    backend: RasterBackend,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Option<PreviewOverride>> {
    let Some(preview_path) = response.preview_path.clone() else {
        if matches!(backend, RasterBackend::Photoshop) {
            return Err(AppError::Photoshop(
                "Photoshop export did not return a preview path".to_string(),
            ));
        }
        warnings.push(ExportWarning::new(
            "photoshop-preview-fallback",
            "Photoshop export did not return a preview path, so preview/document.png fell back to rawpsd",
        ));
        return Ok(None);
    };

    let (Some(width), Some(height)) = (response.preview_width, response.preview_height) else {
        if matches!(backend, RasterBackend::Photoshop) {
            return Err(AppError::Photoshop(
                "Photoshop export did not return preview dimensions".to_string(),
            ));
        }
        warnings.push(ExportWarning::new(
            "photoshop-preview-fallback",
            "Photoshop export did not return preview dimensions, so preview/document.png fell back to rawpsd",
        ));
        return Ok(None);
    };

    let staged_path = staging_dir.join(preview_path.replace('/', "\\"));
    if !staged_path.exists() {
        if matches!(backend, RasterBackend::Photoshop) {
            return Err(AppError::Photoshop(format!(
                "Photoshop preview export is missing from staging: {}",
                staged_path.display()
            )));
        }
        warnings.push(ExportWarning::new(
            "photoshop-preview-fallback",
            format!(
                "Photoshop preview export was missing from staging, so preview/document.png fell back to rawpsd: {}",
                staged_path.display()
            ),
        ));
        return Ok(None);
    }

    Ok(Some(PreviewOverride {
        staged_path,
        asset: AssetRef {
            path: preview_path,
            width,
            height,
        },
    }))
}

fn bake_node_effects(nodes: &mut [BuildNode], warnings: &mut Vec<ExportWarning>) {
    for node in nodes {
        if node.external_asset.is_none() {
            if let Some(bitmap) = node.image.as_ref() {
                let outcome = bake_layer_effects(
                    &node.name,
                    bitmap,
                    &node.bounds,
                    node.mask.as_ref(),
                    node.clip_to.as_deref(),
                    &node.effects,
                );

                warnings.extend(outcome.warnings);
                if !outcome.baked.is_empty() {
                    node.effects.baked = outcome.baked;
                }
                if let Some(image) = outcome.image {
                    node.image = Some(image);
                }
                if let Some(bounds) = outcome.bounds {
                    node.bounds = bounds;
                }
            }
        }

        bake_node_effects(&mut node.children, warnings);
    }
}

fn prepare_output_dirs(out_dir: &Path) -> Result<()> {
    for dir in [
        out_dir.to_path_buf(),
        out_dir.join("images"),
        out_dir.join("masks"),
        out_dir.join("preview"),
    ] {
        fs::create_dir_all(&dir).map_err(|error| AppError::io(&dir, error))?;
    }
    Ok(())
}

fn write_preview(out_dir: &Path, bitmap: &RgbaBitmap) -> Result<AssetRef> {
    let relative_path = "preview/document.png".to_string();
    let full_path = out_dir.join("preview").join("document.png");
    write_rgba_bitmap(&full_path, bitmap)?;
    Ok(AssetRef {
        path: relative_path,
        width: bitmap.width,
        height: bitmap.height,
    })
}

fn write_nodes(
    out_dir: &Path,
    nodes: &mut [BuildNode],
    include_hidden: bool,
) -> Result<Vec<LayerNode>> {
    nodes
        .iter_mut()
        .map(|node| write_single_node(out_dir, node, include_hidden))
        .collect()
}

fn write_single_node(
    out_dir: &Path,
    node: &mut BuildNode,
    include_hidden: bool,
) -> Result<LayerNode> {
    let asset = if should_export_image(node, include_hidden) {
        if let Some(asset) = &node.external_asset {
            let full_path = out_dir.join(asset.relative_path.replace('/', "\\"));
            copy_staged_asset(&asset.staged_path, &full_path)?;
            Some(AssetRef {
                path: asset.relative_path.clone(),
                width: asset.width,
                height: asset.height,
            })
        } else {
            let relative_path = asset_relative_path(node);
            let full_path = out_dir.join(relative_path.replace('/', "\\"));
            let image = node.image.as_ref().unwrap();
            write_rgba_bitmap(&full_path, image)?;
            Some(AssetRef {
                path: relative_path,
                width: image.width,
                height: image.height,
            })
        }
    } else {
        None
    };

    let mask = if let Some(mask_bitmap) = &node.mask {
        let relative_path = format!("masks/{}-{}.png", node.id, slugify(&node.name));
        let full_path = out_dir.join(relative_path.replace('/', "\\"));
        write_mask_bitmap(&full_path, mask_bitmap)?;
        Some(MaskRef {
            path: relative_path,
            bounds: mask_bitmap.bounds.clone(),
            default_color: mask_bitmap.default_color,
            relative: mask_bitmap.relative,
            disabled: mask_bitmap.disabled,
            invert: mask_bitmap.invert,
        })
    } else {
        None
    };

    let children = write_nodes(out_dir, &mut node.children, include_hidden)?;

    Ok(LayerNode {
        id: node.id.clone(),
        name: node.name.clone(),
        layer_type: node.layer_type,
        visible: node.visible,
        opacity: node.opacity,
        blend_mode: node.blend_mode.clone(),
        bounds: node.bounds.clone(),
        stack_index: node.stack_index,
        children,
        asset,
        mask,
        clip_to: node.clip_to.clone(),
        text: node.text.clone(),
        effects: node.effects.clone(),
        unsupported: node.unsupported.clone(),
    })
}

fn should_export_image(node: &BuildNode, include_hidden: bool) -> bool {
    can_write_image_asset(node)
        && node.bounds.width > 0
        && node.bounds.height > 0
        && (include_hidden || node.visible)
}

fn asset_relative_path(node: &BuildNode) -> String {
    format!("images/{}.png", node.asset_file_stem)
}

fn photoshop_path_string(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let normalized = if let Some(stripped) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{stripped}")
    } else if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        raw.to_string()
    };
    normalized.replace('\\', "/")
}

fn copy_staged_asset(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;
    }
    fs::copy(source, destination).map_err(|error| {
        AppError::Manifest(format!(
            "failed to copy staged Photoshop asset from {} to {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    Ok(())
}

fn write_rgba_bitmap(path: &Path, bitmap: &RgbaBitmap) -> Result<()> {
    let image = RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.pixels.clone())
        .ok_or_else(|| {
            AppError::Manifest(format!("invalid RGBA buffer size for {}", path.display()))
        })?;
    image.save(path)?;
    Ok(())
}

fn write_mask_bitmap(path: &Path, bitmap: &MaskBitmap) -> Result<()> {
    let image: GrayImage = ImageBuffer::from_raw(
        bitmap.bounds.width,
        bitmap.bounds.height,
        bitmap.pixels.clone(),
    )
    .ok_or_else(|| {
        AppError::Manifest(format!("invalid mask buffer size for {}", path.display()))
    })?;
    image.save(path)?;
    Ok(())
}

fn generate_node_id(path: &[usize], raw_index: usize) -> String {
    let path_key = path
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("/");
    let mut hasher = Sha1::new();
    hasher.update(format!("{path_key}#{raw_index}").as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    format!("layer_{}", &hex[..12])
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "layer".to_string()
    } else {
        slug.to_string()
    }
}

fn sanitize_file_stem(name: &str) -> String {
    let mut sanitized = String::new();
    for ch in name.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            sanitized.push('_');
        } else {
            sanitized.push(ch);
        }
    }

    let sanitized = sanitized.trim_matches(|ch| ch == ' ' || ch == '.');
    let sanitized = if sanitized.is_empty() {
        "layer".to_string()
    } else {
        sanitized.to_string()
    };

    if is_windows_reserved_name(&sanitized) {
        format!("{sanitized}_")
    } else {
        sanitized
    }
}

fn is_windows_reserved_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

impl BuildNode {
    fn from_flat_layer(layer: FlatLayer) -> Self {
        let asset_file_stem = sanitize_file_stem(&layer.name);
        Self {
            raw_index: layer.raw_index,
            name: layer.name,
            asset_file_stem,
            layer_type: layer.layer_type,
            visible: layer.visible,
            opacity: layer.opacity,
            blend_mode: layer.blend_mode,
            bounds: layer.bounds,
            children: Vec::new(),
            image: layer.image,
            mask: layer.mask,
            text: layer.text,
            effects: layer.effects,
            unsupported: layer.unsupported,
            is_clipped: layer.is_clipped,
            id: String::new(),
            stack_index: 0,
            clip_to: None,
            path_indices: Vec::new(),
            external_asset: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::*;
    use crate::manifest::{ColorRgba, DropShadowEffect, LayerEffects, StrokeEffect};

    #[test]
    fn exports_rawpsd_fixture_end_to_end() {
        let fixture = find_rawpsd_fixture("test2.psd").expect("rawpsd fixture not found");
        let out_dir = tempdir().unwrap();

        let manifest = export_psd_file(
            &fixture,
            ExportOptions {
                out_dir: out_dir.path().join("sample"),
                include_hidden: false,
                with_preview: true,
                strict: false,
                raster_backend: RasterBackend::RawPsd,
                photoshop_exe: None,
                photoshop_timeout_sec: 120,
            },
        )
        .unwrap();

        assert_eq!(manifest.schema_version, "2.1.0");
        assert_eq!(manifest.layers.len(), 2);
        assert!(out_dir.path().join("sample").join("manifest.json").exists());
        assert!(
            out_dir
                .path()
                .join("sample")
                .join("preview")
                .join("document.png")
                .exists()
        );
        assert!(out_dir.path().join("sample").join("images").exists());
    }

    #[test]
    fn baked_effects_update_asset_size_and_bounds() {
        let out_dir = tempdir().unwrap();
        let mut node = sample_build_node("Badge");
        node.effects = LayerEffects {
            stroke: Some(StrokeEffect {
                color: Some(ColorRgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                opacity: Some(1.0),
                size: Some(1.0),
                position: Some("outside".to_string()),
                blend_mode: None,
                enabled: true,
            }),
            ..LayerEffects::default()
        };
        let mut warnings = Vec::new();

        bake_node_effects(std::slice::from_mut(&mut node), &mut warnings);
        prepare_output_dirs(out_dir.path()).unwrap();
        let layer = write_single_node(out_dir.path(), &mut node, false).unwrap();

        assert!(warnings.is_empty());
        assert_eq!(layer.bounds.x, 9);
        assert_eq!(layer.bounds.y, 19);
        assert_eq!(layer.bounds.width, 3);
        assert_eq!(layer.bounds.height, 3);
        assert_eq!(layer.effects.baked, vec!["stroke".to_string()]);
        let asset = layer.asset.expect("asset should exist");
        assert_eq!(asset.width, 3);
        assert_eq!(asset.height, 3);
        assert!(out_dir.path().join(asset.path.replace('/', "\\")).exists());
    }

    #[test]
    fn baking_skips_clipped_layers_with_warning() {
        let mut node = sample_build_node("Shadowed");
        node.clip_to = Some("layer_base".to_string());
        node.effects = LayerEffects {
            drop_shadow: Some(DropShadowEffect {
                color: Some(ColorRgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                opacity: Some(0.5),
                blur: Some(2.0),
                distance: Some(4.0),
                angle: Some(45.0),
                blend_mode: Some("multiply".to_string()),
                enabled: true,
            }),
            ..LayerEffects::default()
        };
        let mut warnings = Vec::new();

        bake_node_effects(std::slice::from_mut(&mut node), &mut warnings);

        assert_eq!(node.bounds.width, 1);
        assert_eq!(node.bounds.height, 1);
        assert!(node.effects.baked.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "effects-bake-skipped-clipping");
    }

    #[test]
    fn photoshop_merge_updates_asset_size_and_bounds() {
        let temp = tempdir().unwrap();
        let staging_dir = temp.path().join(".photoshop-stage");
        let staged_path = staging_dir.join("images").join("layer_test-badge.png");
        fs::create_dir_all(staged_path.parent().unwrap()).unwrap();
        fs::write(&staged_path, b"png-data").unwrap();

        let mut node = sample_build_node("Badge");
        let requests = vec![PhotoshopLayerExportRequest {
            id: node.id.clone(),
            name: node.name.clone(),
            raw_index: node.raw_index,
            path_indices: vec![0],
            expected_visible: true,
            output_png_relpath: "images/layer_test-badge.png".to_string(),
        }];
        let response = PhotoshopExportResponse {
            preview_path: None,
            preview_width: None,
            preview_height: None,
            warnings: Vec::new(),
            layers: vec![PhotoshopLayerExportResult {
                id: node.id.clone(),
                exported: true,
                bounds: Some(Bounds {
                    x: 2,
                    y: 3,
                    width: 11,
                    height: 12,
                }),
                width: Some(11),
                height: Some(12),
                warnings: Vec::new(),
            }],
        };
        let mut warnings = Vec::new();

        let preview = apply_photoshop_response(
            std::slice::from_mut(&mut node),
            &requests,
            &response,
            &staging_dir,
            RasterBackend::Auto,
            &mut warnings,
        )
        .unwrap();

        assert!(preview.is_none());
        assert!(
            warnings
                .iter()
                .any(|warning| warning.code == "photoshop-preview-fallback")
        );
        assert_eq!(node.bounds.x, 2);
        assert_eq!(node.bounds.y, 3);
        assert_eq!(node.bounds.width, 11);
        assert_eq!(node.bounds.height, 12);
        assert_eq!(node.effects.baked, vec!["photoshop_raster".to_string()]);
        assert!(node.external_asset.is_some());
    }

    #[test]
    fn auto_backend_falls_back_on_failed_layer_export() {
        let temp = tempdir().unwrap();
        let mut node = sample_build_node("Badge");
        let requests = vec![PhotoshopLayerExportRequest {
            id: node.id.clone(),
            name: node.name.clone(),
            raw_index: node.raw_index,
            path_indices: vec![0],
            expected_visible: true,
            output_png_relpath: "images/layer_test-badge.png".to_string(),
        }];
        let response = PhotoshopExportResponse {
            preview_path: None,
            preview_width: None,
            preview_height: None,
            warnings: Vec::new(),
            layers: vec![PhotoshopLayerExportResult {
                id: node.id.clone(),
                exported: false,
                bounds: None,
                width: None,
                height: None,
                warnings: vec!["render failed".to_string()],
            }],
        };
        let mut warnings = Vec::new();

        let preview = apply_photoshop_response(
            std::slice::from_mut(&mut node),
            &requests,
            &response,
            &temp.path().join(".photoshop-stage"),
            RasterBackend::Auto,
            &mut warnings,
        )
        .unwrap();

        assert!(preview.is_none());
        assert!(node.external_asset.is_none());
        assert!(
            warnings
                .iter()
                .any(|warning| warning.code == "photoshop-layer-fallback")
        );
    }

    #[test]
    fn photoshop_requests_include_pixel_and_shape_layers_but_exclude_text() {
        let mut text_node = sample_build_node("Title");
        text_node.layer_type = LayerType::Text;
        text_node.text = Some(crate::manifest::TextInfo {
            content: "Title".to_string(),
            character_runs: Vec::new(),
            paragraph_runs: Vec::new(),
            font_family: None,
            font_size: None,
            color: None,
            alignment: None,
        });
        let mut shape_node = sample_build_node("Badge Frame");
        shape_node.layer_type = LayerType::Shape;
        shape_node.image = None;

        let requests = collect_photoshop_layer_requests(
            &[sample_build_node("Badge"), shape_node, text_node],
            false,
        );

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].name, "Badge");
        assert_eq!(requests[0].output_png_relpath, "images/Badge.png");
        assert_eq!(requests[1].name, "Badge Frame");
        assert_eq!(requests[1].output_png_relpath, "images/Badge Frame.png");
    }

    #[test]
    fn non_pixel_layers_do_not_write_assets() {
        let out_dir = tempdir().unwrap();
        prepare_output_dirs(out_dir.path()).unwrap();

        let mut text_node = sample_build_node("Title");
        text_node.layer_type = LayerType::Text;
        text_node.text = Some(crate::manifest::TextInfo {
            content: "Title".to_string(),
            character_runs: Vec::new(),
            paragraph_runs: Vec::new(),
            font_family: None,
            font_size: None,
            color: None,
            alignment: None,
        });

        let layer = write_single_node(out_dir.path(), &mut text_node, false).unwrap();

        assert!(layer.asset.is_none());
    }

    #[test]
    fn shape_layers_write_assets_when_photoshop_stages_pngs() {
        let out_dir = tempdir().unwrap();
        prepare_output_dirs(out_dir.path()).unwrap();

        let staged_path = out_dir
            .path()
            .join("staging")
            .join("images")
            .join("Badge Frame.png");
        fs::create_dir_all(staged_path.parent().unwrap()).unwrap();
        fs::write(&staged_path, b"png-data").unwrap();

        let mut shape_node = sample_build_node("Badge Frame");
        shape_node.layer_type = LayerType::Shape;
        shape_node.image = None;
        shape_node.external_asset = Some(StagedAsset {
            staged_path,
            relative_path: "images/Badge Frame.png".to_string(),
            width: 16,
            height: 16,
        });

        let layer = write_single_node(out_dir.path(), &mut shape_node, false).unwrap();

        let asset = layer
            .asset
            .expect("shape layer should emit an asset when Photoshop staged it");
        assert_eq!(asset.path, "images/Badge Frame.png");
        assert_eq!(asset.width, 16);
        assert_eq!(asset.height, 16);
        assert!(
            out_dir
                .path()
                .join("images")
                .join("Badge Frame.png")
                .exists()
        );
    }

    #[test]
    fn duplicate_layer_names_get_unique_asset_paths() {
        let out_dir = tempdir().unwrap();
        prepare_output_dirs(out_dir.path()).unwrap();

        let mut nodes = vec![sample_build_node("Badge"), sample_build_node("Badge")];
        assign_ids_and_stack_indices(&mut nodes, &mut Vec::new());
        assign_asset_file_stems(&mut nodes);

        let layers = write_nodes(out_dir.path(), &mut nodes, false).unwrap();

        assert_eq!(layers[0].asset.as_ref().unwrap().path, "images/Badge.png");
        assert_eq!(
            layers[1].asset.as_ref().unwrap().path,
            "images/Badge (2).png"
        );
        assert!(out_dir.path().join("images").join("Badge.png").exists());
        assert!(out_dir.path().join("images").join("Badge (2).png").exists());
    }

    #[test]
    fn photoshop_backend_errors_on_failed_layer_export() {
        let temp = tempdir().unwrap();
        let mut node = sample_build_node("Badge");
        let requests = vec![PhotoshopLayerExportRequest {
            id: node.id.clone(),
            name: node.name.clone(),
            raw_index: node.raw_index,
            path_indices: vec![0],
            expected_visible: true,
            output_png_relpath: "images/layer_test-badge.png".to_string(),
        }];
        let response = PhotoshopExportResponse {
            preview_path: Some("preview/document.png".to_string()),
            preview_width: Some(100),
            preview_height: Some(100),
            warnings: Vec::new(),
            layers: vec![PhotoshopLayerExportResult {
                id: node.id.clone(),
                exported: false,
                bounds: None,
                width: None,
                height: None,
                warnings: Vec::new(),
            }],
        };

        let error = apply_photoshop_response(
            std::slice::from_mut(&mut node),
            &requests,
            &response,
            &temp.path().join(".photoshop-stage"),
            RasterBackend::Photoshop,
            &mut Vec::new(),
        )
        .unwrap_err();

        assert!(matches!(error, AppError::Photoshop(_)));
    }

    fn find_rawpsd_fixture(file_name: &str) -> Option<PathBuf> {
        let cargo_home = std::env::var("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("USERPROFILE")
                    .map(PathBuf::from)
                    .unwrap()
                    .join(".cargo")
            });
        let registry_src = cargo_home.join("registry").join("src");
        find_fixture_recursive(&registry_src, file_name)
    }

    fn find_fixture_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
        let entries = fs::read_dir(root).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .map(|name| name.to_string_lossy().contains("rawpsd-0.2.2"))
                    .unwrap_or(false)
                {
                    let candidate = path.join("data").join(file_name);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
                if let Some(found) = find_fixture_recursive(&path, file_name) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn sample_build_node(name: &str) -> BuildNode {
        BuildNode {
            raw_index: 0,
            name: name.to_string(),
            asset_file_stem: sanitize_file_stem(name),
            layer_type: LayerType::Pixel,
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: Bounds {
                x: 10,
                y: 20,
                width: 1,
                height: 1,
            },
            children: Vec::new(),
            image: Some(RgbaBitmap {
                width: 1,
                height: 1,
                pixels: vec![255, 255, 255, 255],
            }),
            mask: None,
            text: None,
            effects: LayerEffects::default(),
            unsupported: Vec::new(),
            is_clipped: false,
            id: "layer_test".to_string(),
            stack_index: 0,
            clip_to: None,
            path_indices: vec![0],
            external_asset: None,
        }
    }
}
