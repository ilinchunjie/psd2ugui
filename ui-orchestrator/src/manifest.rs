use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{OrchestratorError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: String,
    pub source: ManifestSource,
    pub document: ManifestDocument,
    #[serde(default)]
    pub warnings: Vec<ManifestWarning>,
    #[serde(default)]
    pub layers: Vec<ManifestLayer>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestSource {
    pub input_path: String,
    pub input_file: String,
    pub file_size: u64,
    pub file_sha1: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestDocument {
    pub width: i32,
    pub height: i32,
    pub color_mode: String,
    pub depth: i32,
    pub channel_count: i32,
    pub preview: ManifestPreview,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestPreview {
    pub path: String,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestLayer {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub layer_type: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default)]
    pub blend_mode: String,
    pub bounds: ManifestBounds,
    #[serde(default)]
    pub stack_index: i32,
    #[serde(default)]
    pub children: Vec<ManifestLayer>,
    #[serde(default)]
    pub asset: Option<ManifestAsset>,
    #[serde(default)]
    pub mask: Option<ManifestMask>,
    #[serde(default)]
    pub clip_to: Option<String>,
    #[serde(default)]
    pub text: Option<ManifestText>,
    #[serde(default)]
    pub effects: Value,
    #[serde(default)]
    pub unsupported: Vec<ManifestUnsupported>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestAsset {
    pub path: String,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestMask {
    pub path: String,
    pub bounds: ManifestBounds,
    #[serde(default)]
    pub default_color: Option<i32>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ManifestText {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub character_runs: Vec<ManifestCharacterRun>,
    #[serde(default)]
    pub paragraph_runs: Vec<ManifestParagraphRun>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub color: Option<ManifestColor>,
    #[serde(default)]
    pub alignment: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestCharacterRun {
    pub start: usize,
    pub length: usize,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_style: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub color: Option<ManifestColor>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestParagraphRun {
    pub start: usize,
    pub length: usize,
    #[serde(default)]
    pub alignment: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    #[serde(default = "default_alpha")]
    pub a: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestUnsupported {
    pub kind: String,
    pub reason: String,
}

fn default_true() -> bool {
    true
}

fn default_opacity() -> f32 {
    1.0
}

fn default_alpha() -> u8 {
    255
}

pub fn load_manifest(bundle_dir: &Path) -> Result<(Manifest, PathBuf)> {
    if !bundle_dir.exists() {
        return Err(OrchestratorError::MissingBundle(bundle_dir.to_path_buf()));
    }

    let manifest_path = bundle_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Err(OrchestratorError::MissingFile(manifest_path));
    }

    let contents =
        fs::read_to_string(&manifest_path).map_err(|source| OrchestratorError::ReadFile {
            path: manifest_path.clone(),
            source,
        })?;

    let manifest =
        serde_json::from_str(&contents).map_err(|source| OrchestratorError::ParseJson {
            path: manifest_path.clone(),
            source,
        })?;

    Ok((manifest, manifest_path))
}
