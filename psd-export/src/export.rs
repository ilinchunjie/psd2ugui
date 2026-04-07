use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

use crate::error::{AppError, Result};
use crate::manifest::{
    AssetRef, Bounds, DocumentInfo, ExportManifest, ExportWarning, LayerNode, LayerType, SourceInfo,
};
use crate::photoshop::{
    self, PhotoshopExportOptions, PhotoshopExportRequest, PhotoshopExportResponse,
    PhotoshopLayerExportRequest, PhotoshopLayerExportResult,
};
use crate::psd::{self, FlatLayer, ParseOptions};

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub out_dir: PathBuf,
    pub include_hidden: bool,
    pub with_preview: bool,
    pub strict: bool,
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

#[derive(Debug)]
struct ReconstructionResult {
    nodes: Vec<BuildNode>,
    synthetic_opener_count: usize,
    orphan_opener_count: usize,
    orphan_closer_count: usize,
}

pub fn export_psd_file(input: &Path, options: ExportOptions) -> Result<ExportManifest> {
    let parsed = psd::parse_psd(
        input,
        ParseOptions {
            strict: options.strict,
        },
    )?;

    let mut warnings = parsed.warnings.clone();
    let mut nodes = build_layer_tree(parsed.layers, &mut warnings)?;
    assign_ids_and_stack_indices(&mut nodes, &mut Vec::new());
    assign_clip_targets(&mut nodes, &mut warnings);
    assign_asset_file_stems(&mut nodes);

    let preview_override = rasterize_with_photoshop(input, &options, &mut nodes, &mut warnings)?;

    if options.strict && !warnings.is_empty() {
        return Err(AppError::StrictWarnings {
            warning_count: warnings.len(),
        });
    }

    prepare_output_dirs(&options.out_dir)?;

    let preview_asset = preview_override
        .map(|preview| -> Result<AssetRef> {
            copy_staged_asset(
                &preview.staged_path,
                &options.out_dir.join("preview").join("document.png"),
            )?;
            Ok(preview.asset)
        })
        .transpose()?
        .ok_or_else(|| {
            AppError::Manifest(
                "Photoshop did not produce a preview image; export cannot produce preview/document.png".to_string(),
            )
        })?;

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

    let response = photoshop::run_photoshop_export(
        &request,
        &PhotoshopExportOptions {
            photoshop_exe: options.photoshop_exe.clone(),
            timeout_sec: options.photoshop_timeout_sec,
        },
        &options.out_dir,
    )?;

    apply_photoshop_response(
        nodes,
        &requests,
        &response,
        &staging_dir,
        warnings,
    )
}

fn build_layer_tree(
    layers: Vec<FlatLayer>,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Vec<BuildNode>> {
    let synthetic_closer_count = layers
        .iter()
        .filter(|layer| is_synthetic_group_closer(layer))
        .count();

    match reconstruct_bottom_up(layers.clone()) {
        Ok(result) => {
            push_synthetic_group_closer_warning(synthetic_closer_count, warnings);
            push_synthetic_group_opener_warning(result.synthetic_opener_count, warnings);
            push_orphan_group_opener_warning(result.orphan_opener_count, warnings);
            push_orphan_group_closer_warning(result.orphan_closer_count, warnings);
            Ok(result.nodes)
        }
        Err(primary_error) => match reconstruct_bottom_up(layers.into_iter().rev().collect()) {
            Ok(result) => {
                push_synthetic_group_closer_warning(synthetic_closer_count, warnings);
                push_synthetic_group_opener_warning(result.synthetic_opener_count, warnings);
                push_orphan_group_opener_warning(result.orphan_opener_count, warnings);
                push_orphan_group_closer_warning(result.orphan_closer_count, warnings);
                warnings.push(ExportWarning::new(
                    "layer-order-fallback",
                    format!(
                        "used reversed raw layer order while rebuilding groups: {primary_error}"
                    ),
                ));
                Ok(result.nodes)
            }
            Err(secondary_error) => Err(AppError::Manifest(format!(
                "failed to rebuild PSD layer hierarchy: {primary_error}; reversed fallback also failed: {secondary_error}"
            ))),
        },
    }
}

fn push_synthetic_group_closer_warning(
    synthetic_closer_count: usize,
    warnings: &mut Vec<ExportWarning>,
) {
    if synthetic_closer_count == 0 {
        return;
    }

    warnings.push(ExportWarning::new(
        "synthetic-group-closer",
        format!(
            "recovered {synthetic_closer_count} unflagged '</Layer group>' sentinel layer(s) while rebuilding groups"
        ),
    ));
}

fn push_synthetic_group_opener_warning(
    synthetic_opener_count: usize,
    warnings: &mut Vec<ExportWarning>,
) {
    if synthetic_opener_count == 0 {
        return;
    }

    warnings.push(ExportWarning::new(
        "synthetic-group-opener",
        format!(
            "recovered {synthetic_opener_count} unflagged zero-bounds group opener layer(s) while rebuilding groups"
        ),
    ));
}

fn push_orphan_group_opener_warning(
    orphan_opener_count: usize,
    warnings: &mut Vec<ExportWarning>,
) {
    if orphan_opener_count == 0 {
        return;
    }

    warnings.push(ExportWarning::new(
        "orphan-group-opener",
        format!(
            "recovered {orphan_opener_count} group opener layer(s) that had no matching closer; treated as childless group(s)"
        ),
    ));
}

fn push_orphan_group_closer_warning(
    orphan_closer_count: usize,
    warnings: &mut Vec<ExportWarning>,
) {
    if orphan_closer_count == 0 {
        return;
    }

    warnings.push(ExportWarning::new(
        "orphan-group-closer",
        format!(
            "recovered {orphan_closer_count} group closer layer(s) that had no matching opener; their children were promoted to the parent level"
        ),
    ));
}

fn is_synthetic_group_closer(layer: &FlatLayer) -> bool {
    !layer.group_opener
        && !layer.group_closer
        && layer.bounds.width == 0
        && layer.bounds.height == 0
        && layer.name == "</Layer group>"
}

fn is_synthetic_group_opener_candidate(layer: &FlatLayer) -> bool {
    !layer.group_opener
        && !layer.group_closer
        && layer.bounds.width == 0
        && layer.bounds.height == 0
        && layer.name != "</Layer group>"
}

fn reconstruct_bottom_up(
    layers: Vec<FlatLayer>,
) -> std::result::Result<ReconstructionResult, String> {
    let mut stack: Vec<Vec<BuildNode>> = vec![Vec::new()];
    let mut future_closer_counts = vec![0isize; layers.len() + 1];
    let mut future_opener_counts = vec![0isize; layers.len() + 1];
    let mut future_candidate_opener_counts = vec![0isize; layers.len() + 1];
    let mut synthetic_opener_count = 0usize;
    let mut orphan_opener_count = 0usize;

    for index in (0..layers.len()).rev() {
        let layer = &layers[index];
        future_closer_counts[index] = future_closer_counts[index + 1]
            + if layer.group_closer || is_synthetic_group_closer(layer) {
                1
            } else {
                0
            };
        future_opener_counts[index] = future_opener_counts[index + 1]
            + if layer.group_opener { 1 } else { 0 };
        future_candidate_opener_counts[index] = future_candidate_opener_counts[index + 1]
            + if is_synthetic_group_opener_candidate(layer) {
                1
            } else {
                0
            };
    }

    for (index, layer) in layers.into_iter().enumerate() {
        let effective_closer = layer.group_closer || is_synthetic_group_closer(&layer);
        let synthetic_opener_candidate = is_synthetic_group_opener_candidate(&layer);

        if effective_closer {
            stack.push(Vec::new());
            continue;
        }

        let effective_opener = if layer.group_opener {
            true
        } else if synthetic_opener_candidate {
            let final_depth_if_skipped = stack.len() as isize
                + future_closer_counts[index + 1]
                - future_opener_counts[index + 1]
                - future_candidate_opener_counts[index + 1];
            final_depth_if_skipped > 1
        } else {
            false
        };

        if effective_opener {
            if stack.len() == 1 {
                // Orphan opener: no matching closer was found.
                // Treat it as a childless group and continue.
                let mut node = BuildNode::from_flat_layer(layer);
                if synthetic_opener_candidate {
                    synthetic_opener_count += 1;
                    node.layer_type = LayerType::Group;
                }
                orphan_opener_count += 1;
                stack
                    .last_mut()
                    .ok_or_else(|| "layer stack missing".to_string())?
                    .push(node);
                continue;
            }

            let children = stack
                .pop()
                .ok_or_else(|| "group stack underflow".to_string())?;
            let mut node = BuildNode::from_flat_layer(layer);
            if synthetic_opener_candidate {
                synthetic_opener_count += 1;
                node.layer_type = LayerType::Group;
            }
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

    // Collapse any remaining orphan stack frames (from closers without matching openers)
    // by merging their contents into the root level.
    let orphan_closer_count = stack.len().saturating_sub(1);
    while stack.len() > 1 {
        let orphan_frame = stack.pop().unwrap();
        stack
            .last_mut()
            .unwrap()
            .extend(orphan_frame);
    }

    Ok(ReconstructionResult {
        nodes: stack.pop().unwrap().into_iter().rev().collect(),
        synthetic_opener_count,
        orphan_opener_count,
        orphan_closer_count,
    })
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
                output_png_relpath: format!("images/{}.png", node.asset_file_stem),
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

fn apply_photoshop_response(
    nodes: &mut [BuildNode],
    requests: &[PhotoshopLayerExportRequest],
    response: &PhotoshopExportResponse,
    staging_dir: &Path,
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
        warnings,
    )?;

    preview_override_from_response(response, staging_dir, warnings)
}

fn apply_photoshop_response_recursive(
    nodes: &mut [BuildNode],
    request_map: &HashMap<String, &PhotoshopLayerExportRequest>,
    result_map: &HashMap<String, &PhotoshopLayerExportResult>,
    staging_dir: &Path,
    warnings: &mut Vec<ExportWarning>,
) -> Result<()> {
    for node in nodes {
        if let Some(request) = request_map.get(&node.id) {
            let result = result_map.get(&node.id).copied();
            apply_photoshop_result_to_node(node, request, result, staging_dir, warnings)?;
        }
        apply_photoshop_response_recursive(
            &mut node.children,
            request_map,
            result_map,
            staging_dir,
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
    warnings: &mut Vec<ExportWarning>,
) -> Result<()> {
    let Some(result) = result else {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: Photoshop response did not include this layer",
            node.name
        )));
    };

    for warning in &result.warnings {
        warnings.push(ExportWarning::for_layer(
            "photoshop-layer-warning",
            format!("Photoshop layer export warning: {warning}"),
            node.name.clone(),
        ));
    }

    if !result.exported {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: Photoshop did not export a PNG for this layer",
            node.name
        )));
    }

    let Some(bounds) = result.bounds.clone() else {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: Photoshop export did not return layer bounds",
            node.name
        )));
    };
    let Some(width) = result.width else {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: Photoshop export did not return layer width",
            node.name
        )));
    };
    let Some(height) = result.height else {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: Photoshop export did not return layer height",
            node.name
        )));
    };

    let staged_path = staging_dir.join(request.output_png_relpath.replace('/', "\\"));
    if !staged_path.exists() {
        return Err(AppError::Photoshop(format!(
            "failed to rasterize layer '{}' via Photoshop: staged PNG is missing: {}",
            node.name, staged_path.display()
        )));
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

fn preview_override_from_response(
    response: &PhotoshopExportResponse,
    staging_dir: &Path,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Option<PreviewOverride>> {
    let Some(preview_path) = response.preview_path.clone() else {
        warnings.push(ExportWarning::new(
            "photoshop-preview-missing",
            "Photoshop export did not return a preview path",
        ));
        return Ok(None);
    };

    let (Some(width), Some(height)) = (response.preview_width, response.preview_height) else {
        warnings.push(ExportWarning::new(
            "photoshop-preview-missing",
            "Photoshop export did not return preview dimensions",
        ));
        return Ok(None);
    };

    let staged_path = staging_dir.join(preview_path.replace('/', "\\"));
    if !staged_path.exists() {
        warnings.push(ExportWarning::new(
            "photoshop-preview-missing",
            format!(
                "Photoshop preview export is missing from staging: {}",
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
            None
        }
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
        mask: None,
        clip_to: node.clip_to.clone(),
        text: node.text.clone(),
        effects: node.effects.clone(),
        unsupported: node.unsupported.clone(),
    })
}

fn should_export_image(node: &BuildNode, include_hidden: bool) -> bool {
    node.external_asset.is_some()
        && node.bounds.width > 0
        && node.bounds.height > 0
        && (include_hidden || node.visible)
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
        BuildNode {
            raw_index: layer.raw_index,
            name: layer.name,
            asset_file_stem: String::new(),
            layer_type: layer.layer_type,
            visible: layer.visible,
            opacity: layer.opacity,
            blend_mode: layer.blend_mode,
            bounds: layer.bounds,
            children: Vec::new(),
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

