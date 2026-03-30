use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{OrchestratorError, Result};
use crate::manifest::{
    Manifest, ManifestBounds, ManifestCharacterRun, ManifestColor, ManifestLayer,
    ManifestParagraphRun, ManifestText, load_manifest,
};
use crate::models::{
    COMPONENT_CONTAINER, COMPONENT_IMAGE, COMPONENT_TEXT, CoverageReport, PLAN_VERSION,
    PlanDocument, PlanNode, PlanRect, RecoveredText, RecoveredTextCharacterRun,
    RecoveredTextParagraphRun, ReviewItem, SourceBundle, TextRecoveryReport, UiPlan,
    ValidationReport,
};
use crate::validation::validate_plan;

pub struct GeneratedPlan {
    pub ui_plan_path: PathBuf,
    pub validation_report_path: PathBuf,
    pub ui_plan: UiPlan,
    pub validation_report: ValidationReport,
}

#[derive(Default)]
struct PlannerState {
    review_items: Vec<ReviewItem>,
    component_summary: BTreeMap<String, usize>,
    visible_layer_ids: BTreeSet<String>,
    mapped_layer_ids: BTreeSet<String>,
    text_recovery: TextRecoveryReport,
}

#[derive(Debug, Clone)]
struct LayerPlan {
    node: PlanNode,
}

#[derive(Debug, Clone)]
struct RecoveredTextCandidate {
    text: RecoveredText,
    needs_review: bool,
}

pub fn generate_bundle(bundle_dir: &Path, out_dir: &Path) -> Result<GeneratedPlan> {
    let (manifest, manifest_path) = load_manifest(bundle_dir)?;
    let preview_path = bundle_dir.join(&manifest.document.preview.path);
    if !preview_path.exists() {
        return Err(OrchestratorError::MissingFile(preview_path));
    }

    let generated = build_plan(&manifest, bundle_dir, &manifest_path)?;
    write_plan(out_dir, &generated)
}

pub fn build_plan(
    manifest: &Manifest,
    bundle_dir: &Path,
    manifest_path: &Path,
) -> Result<(UiPlan, ValidationReport)> {
    let mut state = PlannerState::default();
    collect_visible_layer_ids(&manifest.layers, &mut state.visible_layer_ids);

    let document_id = derive_document_id(bundle_dir);
    let bundle_dir_string = bundle_dir.to_string_lossy().replace('\\', "/");
    let manifest_path_string = manifest_path.to_string_lossy().replace('\\', "/");
    let generated_at = format!(
        "unix:{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0)
    );

    let root_rect = ManifestBounds {
        x: 0,
        y: 0,
        width: manifest.document.width,
        height: manifest.document.height,
    };

    let mut roots = manifest
        .layers
        .iter()
        .filter(|layer| layer.visible)
        .collect::<Vec<_>>();
    roots.sort_by_key(|layer| layer.stack_index);

    let planned_nodes = roots
        .into_iter()
        .map(|layer| plan_layer(layer, &root_rect, true, &mut state))
        .collect::<Vec<_>>();

    let ui_plan = UiPlan {
        plan_version: PLAN_VERSION.to_string(),
        source_bundle: SourceBundle {
            bundle_dir: bundle_dir_string,
            manifest_path: manifest_path_string,
            preview_path: manifest.document.preview.path.clone(),
            document_id,
            generated_at,
        },
        document: PlanDocument {
            width: manifest.document.width,
            height: manifest.document.height,
            preview_path: manifest.document.preview.path.clone(),
        },
        nodes: planned_nodes.into_iter().map(|item| item.node).collect(),
        review_items: state.review_items.clone(),
        warnings: manifest.warnings.clone(),
    };

    let mut unmapped = state
        .visible_layer_ids
        .difference(&state.mapped_layer_ids)
        .cloned()
        .collect::<Vec<_>>();
    unmapped.sort();

    let status =
        if unmapped.is_empty() && state.review_items.is_empty() && manifest.warnings.is_empty() {
            "ok"
        } else if unmapped.is_empty() {
            "warning"
        } else {
            "error"
        };

    let validation_report = ValidationReport {
        plan_status: status.to_string(),
        coverage: CoverageReport {
            visible_layers: state.visible_layer_ids.len(),
            mapped_layers: state.mapped_layer_ids.len(),
            unmapped_layer_ids: unmapped,
        },
        text_recovery: state.text_recovery,
        component_summary: state.component_summary,
        review_count: state.review_items.len(),
        unity_apply_status: "pending_unity_apply".to_string(),
        warnings: manifest.warnings.clone(),
    };

    validate_plan(&ui_plan)?;
    Ok((ui_plan, validation_report))
}

fn write_plan(out_dir: &Path, generated: &(UiPlan, ValidationReport)) -> Result<GeneratedPlan> {
    fs::create_dir_all(out_dir).map_err(|source| OrchestratorError::WriteFile {
        path: out_dir.to_path_buf(),
        source,
    })?;

    let ui_plan_path = out_dir.join("ui_plan.json");
    let validation_report_path = out_dir.join("validation_report.json");

    let ui_plan_json = serde_json::to_string_pretty(&generated.0).expect("ui plan must serialize");
    let report_json =
        serde_json::to_string_pretty(&generated.1).expect("validation report must serialize");

    fs::write(&ui_plan_path, ui_plan_json).map_err(|source| OrchestratorError::WriteFile {
        path: ui_plan_path.clone(),
        source,
    })?;
    fs::write(&validation_report_path, report_json).map_err(|source| {
        OrchestratorError::WriteFile {
            path: validation_report_path.clone(),
            source,
        }
    })?;

    Ok(GeneratedPlan {
        ui_plan_path,
        validation_report_path,
        ui_plan: generated.0.clone(),
        validation_report: generated.1.clone(),
    })
}

fn plan_layer(
    layer: &ManifestLayer,
    parent_bounds: &ManifestBounds,
    is_root: bool,
    state: &mut PlannerState,
) -> LayerPlan {
    let resolved_bounds = resolve_bounds(layer);
    state.mapped_layer_ids.insert(layer.id.clone());

    let mut visible_children = layer
        .children
        .iter()
        .filter(|child| child.visible)
        .collect::<Vec<_>>();
    visible_children.sort_by_key(|child| child.stack_index);

    let component = classify_component(layer, &visible_children, &resolved_bounds, is_root);
    let mut needs_review = false;
    let mut text = None;

    if component == COMPONENT_TEXT {
        if let Some(candidate) = recover_text(layer, &resolved_bounds) {
            needs_review = candidate.needs_review;
            state.text_recovery.recovered_from_manifest += 1;
            state.text_recovery.total_text_nodes += 1;
            if candidate.needs_review {
                state.review_items.push(ReviewItem {
                    kind: "text_review".to_string(),
                    severity: "warning".to_string(),
                    node_id: layer.id.clone(),
                    message: "manifest text payload was missing content and should be reviewed"
                        .to_string(),
                });
            }
            text = Some(candidate.text);
        } else {
            state.text_recovery.placeholder_fallbacks += 1;
            state.review_items.push(ReviewItem {
                kind: "text_recovery_failed".to_string(),
                severity: "warning".to_string(),
                node_id: layer.id.clone(),
                message: "text layer was missing a manifest-backed character run payload"
                    .to_string(),
            });
        }
    }

    if has_unbaked_effects(layer) {
        state.review_items.push(ReviewItem {
            kind: "effects_preserved".to_string(),
            severity: "info".to_string(),
            node_id: layer.id.clone(),
            message: "layer effects were preserved in metadata only".to_string(),
        });
    }

    for unsupported in &layer.unsupported {
        state.review_items.push(ReviewItem {
            kind: unsupported.kind.clone(),
            severity: "warning".to_string(),
            node_id: layer.id.clone(),
            message: unsupported.reason.clone(),
        });
    }

    let child_plans = visible_children
        .into_iter()
        .map(|child| plan_layer(child, &resolved_bounds, false, state))
        .collect::<Vec<_>>();

    let node = PlanNode {
        node_id: layer.id.clone(),
        name: layer.name.clone(),
        source_layer_ids: vec![layer.id.clone()],
        component_type: component.to_string(),
        rect: plan_rect(&resolved_bounds, parent_bounds),
        render_order: layer.stack_index,
        children: child_plans.into_iter().map(|item| item.node).collect(),
        confidence: component_confidence(component, text.as_ref()),
        needs_review,
        metadata: build_metadata(layer, component, &resolved_bounds),
        text,
        interaction: None,
    };

    *state
        .component_summary
        .entry(component.to_string())
        .or_insert(0) += 1;

    LayerPlan { node }
}

fn component_confidence(component: &str, text: Option<&RecoveredText>) -> f32 {
    match component {
        COMPONENT_CONTAINER => 1.0,
        COMPONENT_TEXT => text.map(|item| item.confidence).unwrap_or(0.35),
        COMPONENT_IMAGE => 0.99,
        _ => 0.5,
    }
}

fn build_metadata(
    layer: &ManifestLayer,
    component: &str,
    resolved_bounds: &ManifestBounds,
) -> Value {
    let mask_path = layer.mask.as_ref().map(|mask| mask.path.clone());
    let asset_path = layer.asset.as_ref().map(|asset| asset.path.clone());
    let mask_mode = layer
        .mask
        .as_ref()
        .map(|mask| infer_mask_mode(&mask.bounds, resolved_bounds));

    json!({
        "layer_name": layer.name,
        "layer_type": layer.layer_type,
        "blend_mode": layer.blend_mode,
        "opacity": layer.opacity,
        "asset_path": asset_path,
        "mask_path": mask_path,
        "clip_to": layer.clip_to,
        "effects": layer.effects,
        "unsupported": layer.unsupported,
        "visible": layer.visible,
        "stack_index": layer.stack_index,
        "component_hint": component,
        "bounds_source": bounds_source(layer, resolved_bounds),
        "mask_mode": mask_mode,
        "resolved_bounds": resolved_bounds,
    })
}

fn bounds_source(layer: &ManifestLayer, resolved_bounds: &ManifestBounds) -> &'static str {
    if layer.bounds.width == resolved_bounds.width
        && layer.bounds.height == resolved_bounds.height
        && layer.bounds.x == resolved_bounds.x
        && layer.bounds.y == resolved_bounds.y
    {
        "manifest"
    } else if layer.children.is_empty() {
        "manifest"
    } else {
        "children_union"
    }
}

fn classify_component<'a>(
    layer: &'a ManifestLayer,
    visible_children: &[&'a ManifestLayer],
    resolved_bounds: &ManifestBounds,
    is_root: bool,
) -> &'static str {
    if is_root {
        return COMPONENT_CONTAINER;
    }

    if !visible_children.is_empty() {
        return COMPONENT_CONTAINER;
    }

    if recover_text(layer, resolved_bounds).is_some() {
        return COMPONENT_TEXT;
    }

    COMPONENT_IMAGE
}

fn recover_text(layer: &ManifestLayer, _bounds: &ManifestBounds) -> Option<RecoveredTextCandidate> {
    let text = layer.text.as_ref()?;
    if !manifest_has_character_runs(Some(text)) {
        return None;
    }

    Some(RecoveredTextCandidate {
        text: RecoveredText {
            content: text_content(text).unwrap_or_default().to_string(),
            source: "manifest_text".to_string(),
            confidence: 0.98,
            character_runs: text
                .character_runs
                .iter()
                .map(recover_character_run)
                .collect(),
            paragraph_runs: text
                .paragraph_runs
                .iter()
                .map(recover_paragraph_run)
                .collect(),
            font_size: text.font_size,
            alignment: text.alignment.clone(),
            color: resolved_text_color(text),
        },
        needs_review: text_content(text).is_none(),
    })
}

fn manifest_has_character_runs(text: Option<&ManifestText>) -> bool {
    text.map(|value| !value.character_runs.is_empty())
        .unwrap_or(false)
}

fn text_content(text: &ManifestText) -> Option<&str> {
    text.content
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolved_text_color(text: &ManifestText) -> Option<String> {
    text.color
        .as_ref()
        .or_else(|| text.character_runs.iter().find_map(|run| run.color.as_ref()))
        .map(color_to_hex)
}

fn recover_character_run(run: &ManifestCharacterRun) -> RecoveredTextCharacterRun {
    RecoveredTextCharacterRun {
        start: run.start,
        length: run.length,
        font_family: run.font_family.clone(),
        font_style: run.font_style.clone(),
        font_size: run.font_size,
        color: run.color.as_ref().map(color_to_hex),
    }
}

fn recover_paragraph_run(run: &ManifestParagraphRun) -> RecoveredTextParagraphRun {
    RecoveredTextParagraphRun {
        start: run.start,
        length: run.length,
        alignment: run.alignment.clone(),
    }
}

fn color_to_hex(color: &ManifestColor) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.r, color.g, color.b, color.a
    )
}

fn infer_mask_mode(mask_bounds: &ManifestBounds, resolved_bounds: &ManifestBounds) -> &'static str {
    if mask_bounds.x == resolved_bounds.x
        && mask_bounds.y == resolved_bounds.y
        && mask_bounds.width == resolved_bounds.width
        && mask_bounds.height == resolved_bounds.height
    {
        "rect"
    } else {
        "alpha"
    }
}

fn plan_rect(bounds: &ManifestBounds, parent_bounds: &ManifestBounds) -> PlanRect {
    PlanRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
        local_x: bounds.x - parent_bounds.x,
        local_y: bounds.y - parent_bounds.y,
    }
}

fn resolve_bounds(layer: &ManifestLayer) -> ManifestBounds {
    let has_manifest_size = layer.bounds.width > 0 || layer.bounds.height > 0;
    if has_manifest_size || layer.children.is_empty() {
        if has_manifest_size {
            return layer.bounds.clone();
        }
        if layer.children.is_empty() {
            return layer.bounds.clone();
        }
    }

    let mut union: Option<ManifestBounds> = None;
    for child in layer.children.iter().filter(|child| child.visible) {
        let child_bounds = resolve_bounds(child);
        if child_bounds.width <= 0 && child_bounds.height <= 0 {
            continue;
        }
        union = Some(match union {
            None => child_bounds,
            Some(current) => union_bounds(&current, &child_bounds),
        });
    }

    union.unwrap_or_else(|| layer.bounds.clone())
}

fn union_bounds(a: &ManifestBounds, b: &ManifestBounds) -> ManifestBounds {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    ManifestBounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn collect_visible_layer_ids(layers: &[ManifestLayer], visible_ids: &mut BTreeSet<String>) {
    for layer in layers {
        if !layer.visible {
            continue;
        }
        visible_ids.insert(layer.id.clone());
        collect_visible_layer_ids(&layer.children, visible_ids);
    }
}

fn derive_document_id(bundle_dir: &Path) -> String {
    sanitize_identifier(
        &bundle_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("document")
            .to_lowercase(),
    )
}

fn sanitize_identifier(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
        } else if character == '-' || character == '_' {
            output.push(character);
        } else if character.is_whitespace() {
            output.push('_');
        }
    }

    if output.is_empty() {
        "document".to_string()
    } else {
        output
    }
}

fn has_unbaked_effects(layer: &ManifestLayer) -> bool {
    let Value::Object(map) = &layer.effects else {
        return false;
    };
    if map.is_empty() {
        return false;
    }

    let baked = effect_baked_keys(map.get("baked"));
    for key in ["stroke", "drop_shadow"] {
        if map.get(key).is_some_and(|value| !value.is_null()) && !baked.contains(key) {
            return true;
        }
    }

    map.iter().any(|(key, value)| {
        key != "baked" && key != "stroke" && key != "drop_shadow" && !value.is_null()
    })
}

fn effect_baked_keys(value: Option<&Value>) -> HashSet<&str> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        Manifest, ManifestCharacterRun, ManifestColor, ManifestDocument, ManifestLayer,
        ManifestParagraphRun, ManifestPreview, ManifestSource, ManifestText,
    };

    fn sample_bounds(x: i32, y: i32, width: i32, height: i32) -> ManifestBounds {
        ManifestBounds {
            x,
            y,
            width,
            height,
        }
    }

    fn sample_manifest(layers: Vec<ManifestLayer>) -> Manifest {
        Manifest {
            schema_version: "2.0.0".to_string(),
            source: ManifestSource {
                input_path: "./psd/sample.psd".to_string(),
                input_file: "sample.psd".to_string(),
                file_size: 1,
                file_sha1: "abc".to_string(),
            },
            document: ManifestDocument {
                width: 400,
                height: 300,
                color_mode: "rgb".to_string(),
                depth: 8,
                channel_count: 3,
                preview: ManifestPreview {
                    path: "preview/document.png".to_string(),
                    width: 400,
                    height: 300,
                },
            },
            warnings: Vec::new(),
            layers,
        }
    }

    fn leaf(id: &str, name: &str, layer_type: &str, bounds: ManifestBounds) -> ManifestLayer {
        ManifestLayer {
            id: id.to_string(),
            name: name.to_string(),
            layer_type: layer_type.to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds,
            stack_index: 0,
            children: Vec::new(),
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        }
    }

    fn text_leaf(id: &str, name: &str, content: Option<&str>) -> ManifestLayer {
        let run_length = content.map(|value| value.chars().count()).unwrap_or(5);
        ManifestLayer {
            text: Some(ManifestText {
                content: content.map(str::to_string),
                character_runs: vec![ManifestCharacterRun {
                    start: 0,
                    length: run_length,
                    font_family: Some("MiSans".to_string()),
                    font_style: Some("Regular".to_string()),
                    font_size: Some(20.0),
                    color: Some(ManifestColor {
                        r: 255,
                        g: 255,
                        b: 255,
                        a: 255,
                    }),
                }],
                paragraph_runs: vec![ManifestParagraphRun {
                    start: 0,
                    length: run_length,
                    alignment: Some("center".to_string()),
                }],
                font_family: Some("MiSans".to_string()),
                font_size: Some(20.0),
                color: Some(ManifestColor {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                }),
                alignment: Some("center".to_string()),
                extra: Default::default(),
            }),
            ..leaf(id, name, "text", sample_bounds(0, 0, 120, 24))
        }
    }

    #[test]
    fn resolves_zero_sized_group_bounds_from_children() {
        let group = ManifestLayer {
            id: "group".to_string(),
            name: "Root".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 0, 0),
            stack_index: 0,
            children: vec![
                leaf("a", "Label", "pixel", sample_bounds(10, 20, 50, 10)),
                leaf("b", "Badge", "pixel", sample_bounds(80, 30, 20, 20)),
            ],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let resolved = resolve_bounds(&group);
        assert_eq!(resolved.x, 10);
        assert_eq!(resolved.y, 20);
        assert_eq!(resolved.width, 90);
        assert_eq!(resolved.height, 30);
    }

    #[test]
    fn groups_named_like_buttons_are_still_containers() {
        let button_group = ManifestLayer {
            id: "button".to_string(),
            name: "Claim Button".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(10, 20, 120, 40),
            stack_index: 0,
            children: vec![
                leaf("bg", "Button Bg", "shape", sample_bounds(10, 20, 120, 40)),
                leaf("label", "Claim", "pixel", sample_bounds(40, 28, 50, 18)),
            ],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let root = ManifestLayer {
            id: "screen".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 120),
            stack_index: 0,
            children: vec![button_group],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (plan, _) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");
        assert_eq!(plan.nodes[0].component_type, COMPONENT_CONTAINER);
        assert_eq!(plan.nodes[0].children[0].component_type, COMPONENT_CONTAINER);
    }

    #[test]
    fn masked_groups_are_still_containers() {
        let masked_group = ManifestLayer {
            id: "list".to_string(),
            name: "Reward List".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(10, 20, 160, 80),
            stack_index: 0,
            children: vec![
                leaf("item-1", "Item 1", "pixel", sample_bounds(20, 25, 140, 16)),
                leaf("item-2", "Item 2", "pixel", sample_bounds(20, 45, 140, 16)),
                leaf("item-3", "Item 3", "pixel", sample_bounds(20, 65, 140, 16)),
                leaf("item-4", "Item 4", "pixel", sample_bounds(20, 85, 140, 16)),
            ],
            asset: None,
            mask: Some(crate::manifest::ManifestMask {
                path: "masks/list.png".to_string(),
                bounds: sample_bounds(10, 20, 160, 80),
                default_color: Some(0),
                disabled: Some(false),
                extra: Default::default(),
            }),
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let root = ManifestLayer {
            id: "screen".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 120),
            stack_index: 0,
            children: vec![masked_group],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (plan, _) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");
        assert_eq!(plan.nodes[0].children[0].component_type, COMPONENT_CONTAINER);
    }

    #[test]
    fn manifest_text_requires_character_runs() {
        let mut no_runs = text_leaf("text", "Rewards", Some("Rewards"));
        no_runs.text.as_mut().unwrap().character_runs.clear();
        assert!(recover_text(&no_runs, &sample_bounds(0, 0, 120, 24)).is_none());

        let candidate = recover_text(
            &text_leaf("text", "Rewards", Some("Rewards")),
            &sample_bounds(0, 0, 120, 24),
        )
        .expect("text should be recovered from manifest payload");

        assert_eq!(candidate.text.content, "Rewards");
        assert_eq!(candidate.text.source, "manifest_text");
        assert_eq!(candidate.text.character_runs.len(), 1);
        assert_eq!(candidate.text.paragraph_runs.len(), 1);
    }

    #[test]
    fn recover_text_falls_back_to_first_character_run_color() {
        let mut layer = text_leaf("text", "Rewards", Some("Rewards"));
        let text = layer.text.as_mut().expect("text payload should exist");
        text.color = None;
        text.character_runs[0].color = Some(ManifestColor {
            r: 181,
            g: 187,
            b: 235,
            a: 255,
        });

        let candidate = recover_text(&layer, &sample_bounds(0, 0, 120, 24))
            .expect("text should be recovered from manifest payload");

        assert_eq!(candidate.text.color.as_deref(), Some("#B5BBEBFF"));
    }

    #[test]
    fn recover_text_prefers_top_level_manifest_color() {
        let mut layer = text_leaf("text", "Rewards", Some("Rewards"));
        let text = layer.text.as_mut().expect("text payload should exist");
        text.color = Some(ManifestColor {
            r: 181,
            g: 187,
            b: 235,
            a: 255,
        });
        text.character_runs[0].color = Some(ManifestColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        });

        let candidate = recover_text(&layer, &sample_bounds(0, 0, 120, 24))
            .expect("text should be recovered from manifest payload");

        assert_eq!(candidate.text.color.as_deref(), Some("#B5BBEBFF"));
    }

    #[test]
    fn build_plan_classifies_manifest_text_with_character_runs() {
        let root = ManifestLayer {
            id: "group".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 100),
            stack_index: 0,
            children: vec![text_leaf("title", "Title", Some("Title"))],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (plan, _) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");
        assert_eq!(plan.nodes[0].children[0].component_type, COMPONENT_TEXT);
        assert_eq!(
            plan.nodes[0].children[0]
                .text
                .as_ref()
                .unwrap()
                .character_runs
                .len(),
            1
        );
    }

    #[test]
    fn build_plan_does_not_infer_text_from_layer_name() {
        let root = ManifestLayer {
            id: "group".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 100),
            stack_index: 0,
            children: vec![leaf(
                "title",
                "Title",
                "pixel",
                sample_bounds(20, 20, 80, 18),
            )],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (plan, _) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");
        assert_eq!(
            plan.nodes[0].children[0].component_type,
            COMPONENT_IMAGE
        );
    }

    #[test]
    fn validation_report_marks_unity_as_pending() {
        let root = ManifestLayer {
            id: "group".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 100),
            stack_index: 0,
            children: vec![leaf(
                "title",
                "Title",
                "pixel",
                sample_bounds(20, 20, 80, 18),
            )],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (_, report) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");
        assert_eq!(report.unity_apply_status, "pending_unity_apply");
    }

    #[test]
    fn does_not_flag_fully_baked_effects_for_review() {
        let mut layer = leaf("badge", "Badge", "pixel", sample_bounds(10, 10, 24, 24));
        layer.effects = json!({
            "stroke": {
                "size": 2.0
            },
            "drop_shadow": {
                "distance": 4.0
            },
            "baked": ["stroke", "drop_shadow"]
        });

        let root = ManifestLayer {
            id: "screen".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 100),
            stack_index: 0,
            children: vec![layer],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (plan, _) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");

        assert!(
            !plan
                .review_items
                .iter()
                .any(|item| item.kind == "effects_preserved")
        );
    }

    #[test]
    fn flags_unbaked_effects_for_review() {
        let mut layer = leaf("badge", "Badge", "pixel", sample_bounds(10, 10, 24, 24));
        layer.effects = json!({
            "stroke": {
                "size": 2.0
            },
            "baked": ["drop_shadow"]
        });

        let root = ManifestLayer {
            id: "screen".to_string(),
            name: "Screen".to_string(),
            layer_type: "group".to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: "norm".to_string(),
            bounds: sample_bounds(0, 0, 200, 100),
            stack_index: 0,
            children: vec![layer],
            asset: None,
            mask: None,
            clip_to: None,
            text: None,
            effects: Value::Object(Default::default()),
            unsupported: Vec::new(),
        };

        let manifest = sample_manifest(vec![root]);
        let (plan, _) = build_plan(
            &manifest,
            Path::new("demo"),
            Path::new("demo/manifest.json"),
        )
        .expect("plan should build");

        assert!(
            plan.review_items
                .iter()
                .any(|item| item.kind == "effects_preserved")
        );
    }
}
