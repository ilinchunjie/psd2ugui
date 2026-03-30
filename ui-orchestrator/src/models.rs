use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::ManifestWarning;

pub const PLAN_VERSION: &str = "2.0.0";

pub const COMPONENT_CONTAINER: &str = "Container";
pub const COMPONENT_IMAGE_PLACEHOLDER: &str = "ImagePlaceholder";
pub const COMPONENT_TMP_TEXT: &str = "TMP_Text";
pub const COMPONENT_BUTTON: &str = "Button";
pub const COMPONENT_SCROLL_VIEW: &str = "ScrollView";
pub const COMPONENT_MASK_GROUP: &str = "MaskGroup";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPlan {
    pub plan_version: String,
    pub source_bundle: SourceBundle,
    pub document: PlanDocument,
    pub nodes: Vec<PlanNode>,
    pub review_items: Vec<ReviewItem>,
    pub warnings: Vec<ManifestWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceBundle {
    pub bundle_dir: String,
    pub manifest_path: String,
    pub preview_path: String,
    pub document_id: String,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDocument {
    pub width: i32,
    pub height: i32,
    pub preview_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanNode {
    pub node_id: String,
    pub name: String,
    pub source_layer_ids: Vec<String>,
    pub component_type: String,
    pub rect: PlanRect,
    pub render_order: i32,
    pub children: Vec<PlanNode>,
    pub confidence: f32,
    pub needs_review: bool,
    pub metadata: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<RecoveredText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<InteractionSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub local_x: i32,
    pub local_y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveredText {
    pub content: String,
    pub source: String,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub character_runs: Vec<RecoveredTextCharacterRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paragraph_runs: Vec<RecoveredTextParagraphRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveredTextCharacterRun {
    pub start: usize,
    pub length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveredTextParagraphRun {
    pub start: usize,
    pub length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionSpec {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_rect: Option<PlanRect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewItem {
    pub kind: String,
    pub severity: String,
    pub node_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub plan_status: String,
    pub coverage: CoverageReport,
    pub text_recovery: TextRecoveryReport,
    pub component_summary: BTreeMap<String, usize>,
    pub review_count: usize,
    pub unity_apply_status: String,
    pub warnings: Vec<ManifestWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub visible_layers: usize,
    pub mapped_layers: usize,
    pub unmapped_layer_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TextRecoveryReport {
    pub total_text_nodes: usize,
    pub recovered_from_manifest: usize,
    pub recovered_from_heuristic: usize,
    pub placeholder_fallbacks: usize,
}
