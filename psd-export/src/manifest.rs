use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct ExportManifest {
    pub schema_version: String,
    pub source: SourceInfo,
    pub document: DocumentInfo,
    pub warnings: Vec<ExportWarning>,
    pub layers: Vec<LayerNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub input_path: String,
    pub input_file: String,
    pub file_size: u64,
    pub file_sha1: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentInfo {
    pub width: u32,
    pub height: u32,
    pub color_mode: String,
    pub depth: u16,
    pub channel_count: u16,
    pub preview: AssetRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetRef {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaskRef {
    pub path: String,
    pub bounds: Bounds,
    pub default_color: u8,
    pub relative: bool,
    pub disabled: bool,
    pub invert: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerType {
    Group,
    Pixel,
    Text,
    Shape,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub layer_type: LayerType,
    pub visible: bool,
    pub opacity: f32,
    pub blend_mode: String,
    pub bounds: Bounds,
    pub stack_index: u32,
    pub children: Vec<LayerNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<MaskRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextInfo>,
    pub effects: LayerEffects,
    pub unsupported: Vec<UnsupportedInfo>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LayerEffects {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<FillEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_shadow: Option<DropShadowEffect>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub baked: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FillEffect {
    pub color: ColorRgba,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrokeEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRgba>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropShadowEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRgba>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextInfo {
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub character_runs: Vec<TextCharacterRun>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub paragraph_runs: Vec<TextParagraphRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRgba>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<TextAlignment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextCharacterRun {
    pub start: usize,
    pub length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRgba>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextParagraphRun {
    pub start: usize,
    pub length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<TextAlignment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ColorRgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnsupportedInfo {
    pub kind: String,
    pub reason: String,
}

impl ExportWarning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            layer_name: None,
        }
    }

    pub fn for_layer(
        code: impl Into<String>,
        message: impl Into<String>,
        layer_name: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            layer_name: Some(layer_name.into()),
        }
    }
}
