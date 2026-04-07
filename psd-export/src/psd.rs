use std::collections::BTreeMap;
use std::fs;
use std::path::Path;


use sha1::{Digest, Sha1};

use crate::error::{AppError, Result};
use crate::manifest::{
    Bounds, ColorRgba, DropShadowEffect, ExportWarning, FillEffect, LayerEffects, LayerType,
    StrokeEffect, TextAlignment, TextCharacterRun, TextInfo, TextParagraphRun, UnsupportedInfo,
};

#[derive(Debug, Clone, Copy)]
pub struct ParseOptions {
    pub strict: bool,
}

#[derive(Debug, Clone)]
struct PsdMetadata {
    width: u32,
    height: u32,
    color_mode: u16,
    depth: u16,
    channel_count: u16,
}

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub source: ParsedSource,
    pub metadata: ParsedMetadata,
    pub layers: Vec<FlatLayer>,
    pub warnings: Vec<ExportWarning>,
}

#[derive(Debug, Clone)]
pub struct ParsedSource {
    pub input_path: String,
    pub input_file: String,
    pub file_size: u64,
    pub file_sha1: String,
}

#[derive(Debug, Clone)]
pub struct ParsedMetadata {
    pub width: u32,
    pub height: u32,
    pub color_mode: String,
    pub depth: u16,
    pub channel_count: u16,
}

#[derive(Debug, Clone)]
pub struct FlatLayer {
    pub raw_index: usize,
    pub name: String,
    pub layer_type: LayerType,
    pub visible: bool,
    pub opacity: f32,
    pub blend_mode: String,
    pub bounds: Bounds,
    pub group_opener: bool,
    pub group_closer: bool,
    pub is_clipped: bool,
    pub text: Option<TextInfo>,
    pub effects: LayerEffects,
    pub unsupported: Vec<UnsupportedInfo>,
}


#[derive(Debug, Clone, Default)]
struct LayerScanEntry {
    text: Option<TextInfo>,
    effects: LayerEffects,
    shape_detected: bool,
    smart_object_detected: bool,
    unsupported: Vec<UnsupportedInfo>,
    warnings: Vec<ExportWarning>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Descriptor {
    class_id: String,
    items: Vec<(String, DescriptorValue)>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum DescriptorValue {
    Integer(i32),
    Double(f64),
    UnitFloat { unit: String, value: f64 },
    Boolean(bool),
    Text(String),
    RawData(Vec<u8>),
    Object(Box<Descriptor>),
    Enum { kind: String, value: String },
    List(Vec<DescriptorValue>),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum EngineValue {
    Dict(BTreeMap<String, EngineValue>),
    Array(Vec<EngineValue>),
    String(String),
    Number(f64),
    Name(String),
    Bool(bool),
    Null,
}

#[derive(Clone, Debug, Default)]
struct SliceCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SliceCursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn position(&self) -> u64 {
        self.pos as u64
    }

    fn set_position(&mut self, pos: u64) -> std::result::Result<(), String> {
        let pos = pos as usize;
        if pos > self.buf.len() {
            return Err("unexpected end of stream".to_string());
        }
        self.pos = pos;
        Ok(())
    }

    fn skip(&mut self, len: u64) -> std::result::Result<(), String> {
        self.set_position(self.position() + len)
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn read_exact(&mut self, out: &mut [u8]) -> std::result::Result<(), String> {
        if self.remaining() < out.len() {
            return Err("unexpected end of stream".to_string());
        }
        out.copy_from_slice(&self.buf[self.pos..self.pos + out.len()]);
        self.pos += out.len();
        Ok(())
    }

    fn take(&mut self, len: u64) -> std::result::Result<Self, String> {
        let len = len as usize;
        if self.remaining() < len {
            return Err("unexpected end of stream".to_string());
        }
        let slice = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(Self::new(slice))
    }
}

pub fn parse_psd(input: &Path, options: ParseOptions) -> Result<ParsedDocument> {
    let data = fs::read(input).map_err(|error| AppError::io(input, error))?;
    let metadata = scan_psd_metadata(&data).map_err(AppError::PsdParse)?;

    if metadata.depth != 8 {
        return Err(AppError::UnsupportedPsd(format!(
            "only 8-bit PSD files are supported, found {}-bit",
            metadata.depth
        )));
    }
    if metadata.color_mode != 3 {
        return Err(AppError::UnsupportedPsd(format!(
            "only RGB PSD files are supported, found color mode {}",
            metadata.color_mode
        )));
    }

    let mut warnings = Vec::new();

    let mut layers = match scan_layer_records(&data) {
        Ok(records) => records,
        Err(error) => {
            if options.strict {
                return Err(AppError::PsdParse(error));
            }
            warnings.push(ExportWarning::new(
                "layer-record-scan-failed",
                format!("failed to scan layer records: {error}"),
            ));
            Vec::new()
        }
    };

    let layer_scans = match scan_layer_metadata(&data) {
        Ok(scans) => scans,
        Err(error) => {
            if options.strict {
                return Err(AppError::PsdParse(error));
            }
            warnings.push(ExportWarning::new(
                "layer-metadata-scan-failed",
                format!("failed to inspect additional layer info blocks: {error}"),
            ));
            vec![LayerScanEntry::default(); layers.len()]
        }
    };

    // Merge scan metadata (text, effects, shapes) into layers.
    for (index, layer) in layers.iter_mut().enumerate() {
        if let Some(scan) = layer_scans.get(index).cloned() {
            warnings.extend(scan.warnings);
            if has_text_character_runs(scan.text.as_ref()) {
                layer.layer_type = LayerType::Text;
            } else if scan.shape_detected {
                layer.layer_type = LayerType::Shape;
            }
            layer.text = scan.text;
            layer.effects = scan.effects;
            layer.unsupported = scan.unsupported;
            if scan.smart_object_detected {
                layer.unsupported.push(UnsupportedInfo {
                    kind: "smart_object".to_string(),
                    reason: "smart object metadata was detected, but phase one only preserves the layer node and raster export".to_string(),
                });
            }
        }
    }

    Ok(ParsedDocument {
        source: ParsedSource {
            input_path: normalize_path(input),
            input_file: input
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown.psd".to_string()),
            file_size: data.len() as u64,
            file_sha1: sha1_hex(&data),
        },
        metadata: ParsedMetadata {
            width: metadata.width,
            height: metadata.height,
            color_mode: "rgb".to_string(),
            depth: metadata.depth,
            channel_count: metadata.channel_count,
        },
        layers,
        warnings,
    })
}

fn scan_psd_metadata(data: &[u8]) -> std::result::Result<PsdMetadata, String> {
    let mut cursor = SliceCursor::new(data);

    let signature = read_b4(&mut cursor)?;
    if signature != *b"8BPS" {
        return Err("invalid PSD signature".to_string());
    }

    let version = read_u16(&mut cursor)?;
    if version != 1 {
        return Err("only standard PSD files are supported".to_string());
    }

    cursor.skip(6)?; // reserved
    let channel_count = read_u16(&mut cursor)?;
    let height = read_u32(&mut cursor)?;
    let width = read_u32(&mut cursor)?;
    let depth = read_u16(&mut cursor)?;
    let color_mode = read_u16(&mut cursor)?;

    Ok(PsdMetadata {
        width,
        height,
        color_mode,
        depth,
        channel_count,
    })
}

/// Scans layer records from the PSD binary without decompressing image data.
/// This is used as a fallback when rawpsd fails to parse all layers (e.g. due to
/// unsupported compression formats like ZIP/deflate). The returned FlatLayer
/// entries contain correct metadata but no image data (Photoshop handles rasterization).
fn scan_layer_records(data: &[u8]) -> std::result::Result<Vec<FlatLayer>, String> {
    let mut cursor = SliceCursor::new(data);

    let signature = read_b4(&mut cursor)?;
    if signature != *b"8BPS" {
        return Err("invalid PSD signature".to_string());
    }

    let version = read_u16(&mut cursor)?;
    if version != 1 {
        return Err("only standard PSD files are supported".to_string());
    }

    cursor.skip(6)?;
    cursor.skip(2 + 4 + 4 + 2 + 2)?;

    let color_mode_length = read_u32(&mut cursor)? as u64;
    cursor.skip(color_mode_length)?;

    let image_resources_length = read_u32(&mut cursor)? as u64;
    cursor.skip(image_resources_length)?;

    let layer_mask_length = read_u32(&mut cursor)? as u64;
    if layer_mask_length == 0 {
        return Ok(Vec::new());
    }

    let _layer_mask_end = cursor.position() + layer_mask_length;
    let layer_info_length = read_u32(&mut cursor)? as u64;
    if layer_info_length == 0 {
        return Ok(Vec::new());
    }

    let layer_count = (read_u16(&mut cursor)? as i16).unsigned_abs() as usize;

    // First pass: skip through layer record headers to find where image data starts,
    // collecting channel data lengths for each layer so we can skip image data.
    let mut channel_data_sizes: Vec<Vec<u32>> = Vec::with_capacity(layer_count);
    let records_start = cursor.position();

    for _ in 0..layer_count {
        // bounds: top, left, bottom, right
        cursor.skip(4 * 4)?;
        let channel_count = read_u16(&mut cursor)? as usize;
        let mut sizes = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            cursor.skip(2)?; // channel id
            sizes.push(read_u32(&mut cursor)?);
        }
        channel_data_sizes.push(sizes);

        // blend_mode_signature (4) + blend_mode_key (4) + opacity (1) + clipping (1) + flags (1) + filler (1)
        cursor.skip(4 + 4 + 1 + 1 + 1 + 1)?;
        let extra_len = read_u32(&mut cursor)? as u64;
        cursor.skip(extra_len)?;
    }

    // `cursor` now points to the start of image data.
    // Compute the position after all image data for each layer.
    let mut image_data_pos = cursor.position();
    let mut image_data_ends: Vec<u64> = Vec::with_capacity(layer_count);
    for sizes in &channel_data_sizes {
        for &size in sizes {
            image_data_pos += size as u64;
        }
        image_data_ends.push(image_data_pos);
    }

    // Second pass: re-read layer record headers to extract metadata.
    cursor.set_position(records_start)?;
    let mut layers = Vec::with_capacity(layer_count);

    for raw_index in 0..layer_count {
        let top = read_i32(&mut cursor)?;
        let left = read_i32(&mut cursor)?;
        let bottom = read_i32(&mut cursor)?;
        let right = read_i32(&mut cursor)?;

        let x = left;
        let y = top;
        let w = (right - left).max(0) as u32;
        let h = (bottom - top).max(0) as u32;

        let channel_count = read_u16(&mut cursor)? as u64;
        cursor.skip(channel_count * 6)?; // channel_id (2) + data_length (4) per channel

        let blend_sig = read_b4(&mut cursor)?;
        if blend_sig != *b"8BIM" {
            return Err("invalid blend mode signature in layer record scan".to_string());
        }

        let blend_mode_key = read_b4_string(&mut cursor)?;
        let opacity = read_u8(&mut cursor)? as f32 / 255.0;
        let clipping = read_u8(&mut cursor)?;
        let flags = read_u8(&mut cursor)?;
        let _filler = read_u8(&mut cursor)?;

        let extra_len = read_u32(&mut cursor)? as u64;
        let extra_end = cursor.position() + extra_len;

        // Read mask data length and skip it.
        let mask_data_len = read_u32(&mut cursor)? as u64;
        cursor.skip(mask_data_len)?;

        // Read blend ranges length and skip it.
        let blend_ranges_len = read_u32(&mut cursor)? as u64;
        cursor.skip(blend_ranges_len)?;

        // Read Pascal name.
        let name_len = read_u8(&mut cursor)? as u64;
        let mut name_bytes = vec![0u8; name_len as usize];
        if name_len > 0 {
            cursor.read_exact(&mut name_bytes)?;
        }
        let name_padding = (4 - ((name_len + 1) % 4)) % 4;
        cursor.skip(name_padding)?;
        let mut name = String::from_utf8_lossy(&name_bytes).to_string();

        // Parse additional layer information blocks for group flags and unicode name.
        let mut group_opener = false;
        let mut group_closer = false;
        let mut fill_opacity: Option<f32> = None;

        while cursor.position() < extra_end {
            let sig = read_b4(&mut cursor)?;
            if sig != *b"8BIM" && sig != *b"8B64" {
                // Skip unrecognized block signature, try to continue
                break;
            }

            let key = read_b4_string(&mut cursor)?;
            let block_len = read_u32(&mut cursor)? as u64;
            let block_end = cursor.position() + block_len;

            match key.as_str() {
                "lsct" => {
                    if block_len >= 4 {
                        let kind = read_u32(&mut cursor)?;
                        group_opener = kind == 1 || kind == 2;
                        group_closer = kind == 3;
                    }
                }
                "luni" => {
                    if block_len >= 4 {
                        let char_count = read_u32(&mut cursor)? as usize;
                        let mut utf16 = Vec::with_capacity(char_count);
                        for _ in 0..char_count {
                            if cursor.position() + 2 > block_end {
                                break;
                            }
                            utf16.push(read_u16(&mut cursor)?);
                        }
                        name = String::from_utf16_lossy(&utf16);
                    }
                }
                "iOpa" => {
                    if block_len >= 1 {
                        fill_opacity = Some(read_u8(&mut cursor)? as f32 / 255.0);
                    }
                }
                _ => {}
            }

            cursor.set_position(block_end)?;
        }

        cursor.set_position(extra_end)?;
        let _ = fill_opacity; // reserved for future use

        let is_clipped = clipping != 0;
        let is_visible = (flags & 2) == 0;

        let layer_type = if group_opener {
            LayerType::Group
        } else if w > 0 && h > 0 {
            LayerType::Pixel
        } else {
            LayerType::Unknown
        };

        layers.push(FlatLayer {
            raw_index,
            name: if name.is_empty() {
                format!("Layer {raw_index}")
            } else {
                name
            },
            layer_type,
            visible: is_visible,
            opacity,
            blend_mode: blend_mode_key,
            bounds: Bounds { x, y, width: w, height: h },
            group_opener,
            group_closer,
            is_clipped,

            text: None,
            effects: LayerEffects::default(),
            unsupported: Vec::new(),
        });
    }

    Ok(layers)
}

fn scan_layer_metadata(data: &[u8]) -> std::result::Result<Vec<LayerScanEntry>, String> {
    let mut cursor = SliceCursor::new(data);

    let signature = read_b4(&mut cursor)?;
    if signature != *b"8BPS" {
        return Err("invalid PSD signature".to_string());
    }

    let version = read_u16(&mut cursor)?;
    if version != 1 {
        return Err("only standard PSD files are supported".to_string());
    }

    cursor.skip(6)?;
    cursor.skip(2 + 4 + 4 + 2 + 2)?;

    let color_mode_length = read_u32(&mut cursor)? as u64;
    cursor.skip(color_mode_length)?;

    let image_resources_length = read_u32(&mut cursor)? as u64;
    cursor.skip(image_resources_length)?;

    let layer_mask_length = read_u32(&mut cursor)? as u64;
    if layer_mask_length == 0 {
        return Ok(Vec::new());
    }

    let layer_mask_end = cursor.position() + layer_mask_length;
    let layer_info_length = read_u32(&mut cursor)? as u64;
    if layer_info_length == 0 {
        cursor.set_position(layer_mask_end)?;
        return Ok(Vec::new());
    }

    let layer_info_end = cursor.position() + layer_info_length;
    let layer_count = (read_u16(&mut cursor)? as i16).unsigned_abs() as usize;
    let mut scans = Vec::with_capacity(layer_count);

    for _ in 0..layer_count {
        cursor.skip(4 * 4)?;
        let channel_count = read_u16(&mut cursor)? as u64;
        cursor.skip(channel_count * 6)?;

        let blend_signature = read_b4(&mut cursor)?;
        if blend_signature != *b"8BIM" {
            return Err("invalid layer blend signature".to_string());
        }

        cursor.skip(4)?;
        cursor.skip(4)?;

        let extra_len = read_u32(&mut cursor)? as u64;
        let extra_end = cursor.position() + extra_len;

        let mask_data_len = read_u32(&mut cursor)? as u64;
        cursor.skip(mask_data_len)?;

        let blend_ranges_len = read_u32(&mut cursor)? as u64;
        cursor.skip(blend_ranges_len)?;

        let name_len = read_u8(&mut cursor)? as u64;
        cursor.skip(name_len)?;
        let name_padding = (4 - ((name_len + 1) % 4)) % 4;
        cursor.skip(name_padding)?;

        let mut scan = LayerScanEntry::default();

        while cursor.position() < extra_end {
            let signature = read_b4(&mut cursor)?;
            if signature != *b"8BIM" && signature != *b"8B64" {
                return Err("invalid additional layer info signature".to_string());
            }

            let key = read_b4_string(&mut cursor)?;
            let len = read_u32(&mut cursor)? as u64;
            let mut block = cursor.take(len)?;

            match key.as_str() {
                "TySh" => match parse_type_tool_block(&mut block) {
                    Ok(info) => {
                        if !has_text_character_runs(Some(&info)) {
                            scan.warnings.push(ExportWarning::new(
                                "text-character-runs-missing",
                                "type tool data was detected but no character runs were extracted; layer will not be treated as text",
                            ));
                        } else {
                            if info.content.is_empty() {
                                scan.warnings.push(ExportWarning::new(
                                    "text-content-missing",
                                    "character runs were extracted but text content was empty",
                                ));
                            }
                            scan.text = Some(info);
                        }
                    }
                    Err(error) => scan
                        .warnings
                        .push(ExportWarning::new("text-parse-failed", error)),
                },
                "SoCo" => {
                    scan.shape_detected = true;
                    match parse_fill_descriptor_block(&mut block) {
                        Ok(fill) => scan.effects.fill = Some(fill),
                        Err(error) => scan
                            .warnings
                            .push(ExportWarning::new("shape-fill-parse-failed", error)),
                    }
                }
                "vstk" | "vscg" => {
                    scan.shape_detected = true;
                    match parse_stroke_descriptor_block(&mut block) {
                        Ok(stroke) => scan.effects.stroke = Some(stroke),
                        Err(error) => scan
                            .warnings
                            .push(ExportWarning::new("shape-stroke-parse-failed", error)),
                    }
                }
                "GdFl" | "PtFl" | "vmsk" | "vsms" => {
                    scan.shape_detected = true;
                    scan.unsupported.push(UnsupportedInfo {
                        kind: "shape_layer".to_string(),
                        reason: format!(
                            "shape metadata block '{}' is preserved as a shape hint, but phase one only extracts basic fill and stroke semantics",
                            key
                        ),
                    });
                }
                "lfx2" => match parse_effects_block(&mut block) {
                    Ok(effects) => merge_effects(&mut scan.effects, effects),
                    Err(error) => scan
                        .warnings
                        .push(ExportWarning::new("effects-parse-failed", error)),
                },
                "SoLd" | "lnkD" | "lnk2" | "PlLd" | "sn2P" => {
                    scan.smart_object_detected = true;
                }
                _ => {}
            }
        }

        cursor.set_position(extra_end)?;
        scans.push(scan);
    }

    cursor.set_position(layer_info_end)?;
    cursor.set_position(layer_mask_end)?;
    Ok(scans)
}

fn parse_type_tool_block(cursor: &mut SliceCursor<'_>) -> std::result::Result<TextInfo, String> {
    let _version = read_u16(cursor)?;
    let transform_xx = read_f64(cursor)?;
    let transform_xy = read_f64(cursor)?;
    let transform_yx = read_f64(cursor)?;
    let transform_yy = read_f64(cursor)?;
    let _transform_tx = read_f64(cursor)?;
    let _transform_ty = read_f64(cursor)?;
    let type_tool_font_scale =
        type_tool_font_scale(transform_xx, transform_xy, transform_yx, transform_yy);

    let _text_version = read_u16(cursor)?;
    let _descriptor_version = read_u32(cursor)?;
    let descriptor = read_descriptor(cursor)?;

    let descriptor_content =
        descriptor_text(&descriptor).map(|value| normalize_text_content(&value));
    let engine_root =
        descriptor_engine_data(&descriptor).and_then(|value| parse_engine_data(&value).ok());

    let content = descriptor_content
        .or_else(|| {
            engine_root
                .as_ref()
                .and_then(parse_engine_text_content_from_root)
        })
        .unwrap_or_default();
    let content_len = content.chars().count();
    let character_runs = engine_root
        .as_ref()
        .and_then(|root| parse_engine_character_runs(root, content_len))
        .unwrap_or_default();
    let paragraph_runs = engine_root
        .as_ref()
        .and_then(|root| parse_engine_paragraph_runs(root, content_len))
        .unwrap_or_default();
    let mut text_info = build_text_info(content, character_runs, paragraph_runs);
    if let Some(scale) = type_tool_font_scale {
        scale_text_font_sizes(&mut text_info, scale);
    }

    if text_info.content.is_empty()
        && text_info.character_runs.is_empty()
        && text_info.paragraph_runs.is_empty()
    {
        return Err("type tool descriptor did not expose text content or runs".to_string());
    }

    let _warp_version = read_u16(cursor)?;
    let _warp_descriptor_version = read_u32(cursor)?;
    let _warp_descriptor = read_descriptor(cursor)?;
    let _ = read_i32(cursor)?;
    let _ = read_i32(cursor)?;
    let _ = read_i32(cursor)?;
    let _ = read_i32(cursor)?;
    Ok(text_info)
}

fn type_tool_font_scale(xx: f64, xy: f64, yx: f64, yy: f64) -> Option<f64> {
    let horizontal_scale = xx.hypot(xy);
    let vertical_scale = yx.hypot(yy);
    let scale = if vertical_scale > 0.0 {
        vertical_scale
    } else {
        horizontal_scale
    };

    if scale.is_finite() && scale > 0.0 {
        Some(scale)
    } else {
        None
    }
}

fn scale_text_font_sizes(text_info: &mut TextInfo, scale: f64) {
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 1e-6 {
        return;
    }

    for run in &mut text_info.character_runs {
        if let Some(font_size) = run.font_size {
            run.font_size = Some(normalize_font_size(font_size as f64 * scale));
        }
    }

    text_info.font_size = text_info
        .character_runs
        .first()
        .and_then(|run| run.font_size)
        .or_else(|| {
            text_info
                .font_size
                .map(|font_size| normalize_font_size(font_size as f64 * scale))
        });
}

fn normalize_font_size(value: f64) -> f32 {
    let nearest_integer = value.round();
    let normalized = if (value - nearest_integer).abs() <= 0.01 {
        nearest_integer
    } else {
        (value * 100_000.0).round() / 100_000.0
    };

    normalized as f32
}

fn parse_fill_descriptor_block(
    cursor: &mut SliceCursor<'_>,
) -> std::result::Result<FillEffect, String> {
    let _version = read_u32(cursor)?;
    let descriptor = read_descriptor(cursor)?;
    let color = descriptor
        .items
        .iter()
        .find_map(|(_, value)| descriptor_value_color(value))
        .ok_or_else(|| "solid color descriptor did not contain an RGB color".to_string())?;
    Ok(FillEffect { color })
}

fn parse_stroke_descriptor_block(
    cursor: &mut SliceCursor<'_>,
) -> std::result::Result<StrokeEffect, String> {
    let first = read_u32(cursor)?;
    let descriptor = if first == 16 {
        read_descriptor(cursor)?
    } else {
        let maybe_second = read_u32(cursor)?;
        if maybe_second != 16 {
            return Err("unsupported vector stroke descriptor version".to_string());
        }
        read_descriptor(cursor)?
    };

    Ok(extract_stroke_effect(&descriptor).unwrap_or(StrokeEffect {
        color: None,
        opacity: None,
        size: None,
        position: None,
        blend_mode: None,
        enabled: true,
    }))
}

fn parse_effects_block(cursor: &mut SliceCursor<'_>) -> std::result::Result<LayerEffects, String> {
    let first = read_u32(cursor)?;
    let second = read_u32(cursor)?;
    let descriptor = if first == 0 && second == 16 {
        read_descriptor(cursor)?
    } else if first == 16 {
        read_descriptor(cursor)?
    } else {
        return Err("unsupported layer effects descriptor version".to_string());
    };

    let mut effects = LayerEffects::default();
    if let Some(stroke) = extract_stroke_effect(&descriptor) {
        effects.stroke = Some(stroke);
    }
    if let Some(shadow) = extract_drop_shadow_effect(&descriptor) {
        effects.drop_shadow = Some(shadow);
    }
    Ok(effects)
}

fn merge_effects(base: &mut LayerEffects, overlay: LayerEffects) {
    if overlay.fill.is_some() {
        base.fill = overlay.fill;
    }
    if overlay.stroke.is_some() {
        base.stroke = overlay.stroke;
    }
    if overlay.drop_shadow.is_some() {
        base.drop_shadow = overlay.drop_shadow;
    }
}

fn descriptor_text(descriptor: &Descriptor) -> Option<String> {
    ["Txt ", "textKey"]
        .into_iter()
        .find_map(|key| descriptor_text_value(descriptor, key))
}

fn descriptor_text_value(descriptor: &Descriptor, key: &str) -> Option<String> {
    descriptor
        .items
        .iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (current, DescriptorValue::Text(text)) if current == key => Some(text.clone()),
            _ => None,
        })
}

fn descriptor_engine_data(descriptor: &Descriptor) -> Option<Vec<u8>> {
    descriptor.items.iter().find_map(|(name, value)| {
        if name != "EngineData" {
            return None;
        }

        match value {
            DescriptorValue::Text(text) => Some(text.as_bytes().to_vec()),
            DescriptorValue::RawData(bytes) => Some(trim_trailing_nuls(bytes).to_vec()),
            _ => None,
        }
    })
}

fn extract_stroke_effect(descriptor: &Descriptor) -> Option<StrokeEffect> {
    let stroke_descriptor = descriptor_object_by_keys(descriptor, &["FrFX", "frameFX"])
        .or_else(|| descriptor_object_by_keys(descriptor, &["strokeStyle"]))?;

    Some(StrokeEffect {
        color: descriptor_object_by_keys(stroke_descriptor, &["Clr ", "color"])
            .and_then(descriptor_rgb_color),
        opacity: descriptor_number_by_keys(stroke_descriptor, &["Opct", "strokeStyleOpacity"])
            .map(percent_to_unit),
        size: descriptor_number_by_keys(stroke_descriptor, &["Sz  ", "strokeStyleLineWidth"]),
        position: descriptor_enum_by_keys(stroke_descriptor, &["Styl", "strokeStyleLineAlignment"])
            .map(map_stroke_position),
        blend_mode: descriptor_enum_by_keys(stroke_descriptor, &["Md  ", "strokeStyleBlendMode"]),
        enabled: descriptor_bool_by_keys(stroke_descriptor, &["enab", "strokeEnabled"])
            .unwrap_or(true),
    })
}

fn extract_drop_shadow_effect(descriptor: &Descriptor) -> Option<DropShadowEffect> {
    let shadow_descriptor = descriptor_object_by_keys(descriptor, &["DrSh", "dropShadow"])
        .or_else(|| {
            descriptor_object_by_keys(descriptor, &["DrShMulti"]).and_then(first_object_from_list)
        })?;

    Some(DropShadowEffect {
        color: descriptor_object_by_keys(shadow_descriptor, &["Clr ", "color"])
            .and_then(descriptor_rgb_color),
        opacity: descriptor_number_by_keys(shadow_descriptor, &["Opct"]).map(percent_to_unit),
        blur: descriptor_number_by_keys(shadow_descriptor, &["blur"]),
        distance: descriptor_number_by_keys(shadow_descriptor, &["Dstn", "distance"]),
        angle: descriptor_number_by_keys(shadow_descriptor, &["lagl", "angle"]),
        blend_mode: descriptor_enum_by_keys(shadow_descriptor, &["Md  ", "mode"]),
        enabled: descriptor_bool_by_keys(shadow_descriptor, &["enab"]).unwrap_or(true),
    })
}

fn first_object_from_list(descriptor: &Descriptor) -> Option<&Descriptor> {
    descriptor.items.iter().find_map(|(_, value)| match value {
        DescriptorValue::List(list) => list.iter().find_map(|item| match item {
            DescriptorValue::Object(object) => Some(object.as_ref()),
            _ => None,
        }),
        _ => None,
    })
}

fn descriptor_object_by_keys<'a>(
    descriptor: &'a Descriptor,
    keys: &[&str],
) -> Option<&'a Descriptor> {
    descriptor.items.iter().find_map(|(name, value)| {
        if keys.contains(&name.as_str()) {
            match value {
                DescriptorValue::Object(object) => Some(object.as_ref()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn descriptor_enum_by_keys(descriptor: &Descriptor, keys: &[&str]) -> Option<String> {
    descriptor.items.iter().find_map(|(name, value)| {
        if keys.contains(&name.as_str()) {
            match value {
                DescriptorValue::Enum { value, .. } => Some(value.clone()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn descriptor_bool_by_keys(descriptor: &Descriptor, keys: &[&str]) -> Option<bool> {
    descriptor.items.iter().find_map(|(name, value)| {
        if keys.contains(&name.as_str()) {
            match value {
                DescriptorValue::Boolean(value) => Some(*value),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn descriptor_number_by_keys(descriptor: &Descriptor, keys: &[&str]) -> Option<f32> {
    descriptor.items.iter().find_map(|(name, value)| {
        if keys.contains(&name.as_str()) {
            descriptor_value_number(value)
        } else {
            None
        }
    })
}

fn descriptor_rgb_color(descriptor: &Descriptor) -> Option<ColorRgba> {
    let red = descriptor_number_by_keys(descriptor, &["Rd  ", "red"])?;
    let green = descriptor_number_by_keys(descriptor, &["Grn ", "green"])?;
    let blue = descriptor_number_by_keys(descriptor, &["Bl  ", "blue"])?;
    Some(ColorRgba {
        r: clamp_color(red),
        g: clamp_color(green),
        b: clamp_color(blue),
        a: 255,
    })
}

fn descriptor_value_color(value: &DescriptorValue) -> Option<ColorRgba> {
    match value {
        DescriptorValue::Object(object) => descriptor_rgb_color(object),
        _ => None,
    }
}

fn descriptor_value_number(value: &DescriptorValue) -> Option<f32> {
    match value {
        DescriptorValue::Integer(value) => Some(*value as f32),
        DescriptorValue::Double(value) => Some(*value as f32),
        DescriptorValue::UnitFloat { value, .. } => Some(*value as f32),
        _ => None,
    }
}

fn has_text_character_runs(text: Option<&TextInfo>) -> bool {
    text.map(|value| !value.character_runs.is_empty())
        .unwrap_or(false)
}

fn parse_engine_text_content_from_root(root: &EngineValue) -> Option<String> {
    first_engine_string(
        root,
        &[&["EngineDict", "Editor", "Text"], &["Editor", "Text"]],
    )
    .map(|value| normalize_text_content(&value))
}

fn parse_engine_character_runs(
    root: &EngineValue,
    content_len: usize,
) -> Option<Vec<TextCharacterRun>> {
    let run_array = first_engine_array(
        root,
        &[
            &["EngineDict", "StyleRun", "RunArray"],
            &["StyleRun", "RunArray"],
        ],
    )?;
    let run_lengths = parse_engine_run_lengths(
        root,
        &[
            &["EngineDict", "StyleRun", "RunLengthArray"],
            &["StyleRun", "RunLengthArray"],
        ],
        run_array.len(),
        content_len,
    )?;
    let font_set = first_engine_array(root, &[&["ResourceDict", "FontSet"], &["FontSet"]]);

    let mut runs = Vec::with_capacity(run_lengths.len());
    let mut start = 0usize;
    for (index, length) in run_lengths.into_iter().enumerate() {
        let style_data = run_array
            .get(index)
            .and_then(|value| engine_lookup(value, &["StyleSheet", "StyleSheetData"]));
        let font_entry = style_data
            .and_then(|value| engine_lookup(value, &["Font"]))
            .and_then(component_to_usize)
            .and_then(|font_index| font_set.and_then(|values| values.get(font_index)))
            .or_else(|| font_set.and_then(|values| values.first()));

        runs.push(TextCharacterRun {
            start,
            length,
            font_family: font_entry.and_then(engine_font_family_from_entry),
            font_style: font_entry.and_then(engine_font_style_from_entry),
            font_size: style_data
                .and_then(|value| engine_lookup(value, &["FontSize"]))
                .and_then(component_to_f64)
                .map(|value| value as f32),
            color: style_data
                .and_then(|value| engine_lookup(value, &["FillColor", "Values"]))
                .and_then(engine_color_from_values),
        });
        start += length;
    }

    if runs.is_empty() { None } else { Some(runs) }
}

fn parse_engine_paragraph_runs(
    root: &EngineValue,
    content_len: usize,
) -> Option<Vec<TextParagraphRun>> {
    let run_array = first_engine_array(
        root,
        &[
            &["EngineDict", "ParagraphRun", "RunArray"],
            &["ParagraphRun", "RunArray"],
        ],
    )?;
    let run_lengths = parse_engine_run_lengths(
        root,
        &[
            &["EngineDict", "ParagraphRun", "RunLengthArray"],
            &["ParagraphRun", "RunLengthArray"],
        ],
        run_array.len(),
        content_len,
    )?;

    let mut runs = Vec::with_capacity(run_lengths.len());
    let mut start = 0usize;
    for (index, length) in run_lengths.into_iter().enumerate() {
        let alignment = run_array
            .get(index)
            .and_then(|value| {
                engine_lookup(value, &["ParagraphSheet", "Properties", "Justification"])
            })
            .and_then(component_to_f64)
            .map(engine_alignment_from_value);

        runs.push(TextParagraphRun {
            start,
            length,
            alignment,
        });
        start += length;
    }

    if runs.is_empty() { None } else { Some(runs) }
}

fn parse_engine_run_lengths(
    root: &EngineValue,
    paths: &[&[&str]],
    run_count: usize,
    content_len: usize,
) -> Option<Vec<usize>> {
    if let Some(lengths) = first_engine_array(root, paths) {
        let lengths = lengths
            .iter()
            .map(component_to_usize)
            .collect::<Option<Vec<_>>>()?;
        return Some(normalize_run_lengths(lengths, content_len));
    }

    if run_count == 1 && content_len > 0 {
        return Some(vec![content_len]);
    }

    None
}

fn normalize_run_lengths(lengths: Vec<usize>, content_len: usize) -> Vec<usize> {
    if content_len == 0 {
        return lengths.into_iter().filter(|length| *length > 0).collect();
    }

    let mut remaining = content_len;
    let mut normalized = Vec::with_capacity(lengths.len());
    for length in lengths {
        if remaining == 0 {
            break;
        }
        let clamped = length.min(remaining);
        if clamped > 0 {
            normalized.push(clamped);
            remaining -= clamped;
        }
    }
    normalized
}

fn engine_font_family_from_entry(value: &EngineValue) -> Option<String> {
    engine_lookup(value, &["Name"]).and_then(engine_string_value)
}

fn engine_font_style_from_entry(value: &EngineValue) -> Option<String> {
    engine_lookup(value, &["StyleName"]).and_then(engine_string_value)
}

fn engine_string_value(value: &EngineValue) -> Option<String> {
    match value {
        EngineValue::String(value) | EngineValue::Name(value) => Some(value.clone()),
        _ => None,
    }
}

fn engine_color_from_values(value: &EngineValue) -> Option<ColorRgba> {
    let values = match value {
        EngineValue::Array(values) if values.len() >= 3 => values,
        _ => return None,
    };

    if values.len() > 3 {
        Some(ColorRgba {
            r: scale_engine_color(component_to_f64(&values[1])?),
            g: scale_engine_color(component_to_f64(&values[2])?),
            b: scale_engine_color(component_to_f64(&values[3])?),
            a: scale_engine_color(component_to_f64(&values[0])?),
        })
    } else {
        Some(ColorRgba {
            r: scale_engine_color(component_to_f64(&values[0])?),
            g: scale_engine_color(component_to_f64(&values[1])?),
            b: scale_engine_color(component_to_f64(&values[2])?),
            a: 255,
        })
    }
}

fn build_text_info(
    content: String,
    character_runs: Vec<TextCharacterRun>,
    paragraph_runs: Vec<TextParagraphRun>,
) -> TextInfo {
    let font_family = character_runs
        .first()
        .and_then(|run| run.font_family.clone());
    let font_size = character_runs.first().and_then(|run| run.font_size);
    let color = character_runs.first().and_then(|run| run.color);
    let alignment = paragraph_runs.first().and_then(|run| run.alignment);

    TextInfo {
        content,
        character_runs,
        paragraph_runs,
        font_family,
        font_size,
        color,
        alignment,
    }
}

fn engine_alignment_from_value(value: f64) -> TextAlignment {
    match value as i32 {
        0 => TextAlignment::Left,
        1 => TextAlignment::Right,
        2 => TextAlignment::Center,
        _ => TextAlignment::Justify,
    }
}

fn parse_engine_data(input: impl AsRef<[u8]>) -> std::result::Result<EngineValue, String> {
    let mut parser = EngineParser::new(input.as_ref());
    let value = parser.parse_value()?;
    parser.skip_ws();
    Ok(value)
}

struct EngineParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> EngineParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            bytes: input,
            pos: 0,
        }
    }

    fn parse_value(&mut self) -> std::result::Result<EngineValue, String> {
        self.skip_ws();
        if self.starts_with(b"<<") {
            self.parse_dict()
        } else if self.peek_byte() == Some(b'[') {
            self.parse_array()
        } else if self.peek_byte() == Some(b'(') {
            self.parse_string().map(EngineValue::String)
        } else if self.peek_byte() == Some(b'/') {
            self.parse_name().map(EngineValue::Name)
        } else {
            self.parse_atom()
        }
    }

    fn parse_dict(&mut self) -> std::result::Result<EngineValue, String> {
        self.consume(b"<<")?;
        let mut entries = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.starts_with(b">>") {
                self.consume(b">>")?;
                break;
            }
            let key = self.parse_name()?;
            let value = self.parse_value()?;
            entries.insert(key, value);
        }
        Ok(EngineValue::Dict(entries))
    }

    fn parse_array(&mut self) -> std::result::Result<EngineValue, String> {
        self.expect_byte(b'[')?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_byte() == Some(b']') {
                self.pos += 1;
                break;
            }
            items.push(self.parse_value()?);
        }
        Ok(EngineValue::Array(items))
    }

    fn parse_string(&mut self) -> std::result::Result<String, String> {
        self.expect_byte(b'(')?;
        let mut depth = 1;
        let mut output = Vec::new();

        while self.pos < self.bytes.len() {
            let byte = self
                .next_byte()
                .ok_or_else(|| "unterminated engine string".to_string())?;
            match byte {
                b'\\' => {
                    let escaped = self
                        .next_byte()
                        .ok_or_else(|| "unterminated engine escape".to_string())?;
                    output.push(match escaped {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        other => other,
                    });
                }
                b'(' => {
                    depth += 1;
                    output.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    output.push(byte);
                }
                other => output.push(other),
            }
        }

        Ok(decode_engine_string_bytes(&output))
    }

    fn parse_name(&mut self) -> std::result::Result<String, String> {
        self.expect_byte(b'/')?;
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if is_engine_whitespace(byte) || matches!(byte, b'[' | b']' | b'<' | b'>' | b'(' | b')')
            {
                break;
            }
            self.pos += 1;
        }
        Ok(String::from_utf8_lossy(&self.bytes[start..self.pos]).to_string())
    }

    fn parse_atom(&mut self) -> std::result::Result<EngineValue, String> {
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if is_engine_whitespace(byte) || matches!(byte, b'[' | b']' | b'<' | b'>' | b'(' | b')')
            {
                break;
            }
            self.pos += 1;
        }
        let token = String::from_utf8_lossy(&self.bytes[start..self.pos]).to_string();
        match token.as_str() {
            "true" => Ok(EngineValue::Bool(true)),
            "false" => Ok(EngineValue::Bool(false)),
            "null" => Ok(EngineValue::Null),
            _ => token
                .parse::<f64>()
                .map(EngineValue::Number)
                .or_else(|_| Ok(EngineValue::Name(token))),
        }
    }

    fn skip_ws(&mut self) {
        while let Some(byte) = self.peek_byte() {
            if is_engine_whitespace(byte) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, value: &[u8]) -> std::result::Result<(), String> {
        if self.starts_with(value) {
            self.pos += value.len();
            Ok(())
        } else {
            Err(format!("expected '{}'", String::from_utf8_lossy(value)))
        }
    }

    fn expect_byte(&mut self, byte: u8) -> std::result::Result<(), String> {
        match self.next_byte() {
            Some(current) if current == byte => Ok(()),
            _ => Err(format!("expected '{}'", byte as char)),
        }
    }

    fn starts_with(&self, value: &[u8]) -> bool {
        self.bytes
            .get(self.pos..self.pos + value.len())
            .map(|slice| slice == value)
            .unwrap_or(false)
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.pos += 1;
        Some(byte)
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
}

fn is_engine_whitespace(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == 0
}

fn trim_trailing_nuls(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    &bytes[..end]
}

fn decode_engine_string_bytes(bytes: &[u8]) -> String {
    decode_utf16_engine_string(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).to_string())
}

fn decode_utf16_engine_string(bytes: &[u8]) -> Option<String> {
    let (endianness, raw) = if bytes.starts_with(&[0xFE, 0xFF]) {
        (Utf16Endianness::Big, &bytes[2..])
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (Utf16Endianness::Little, &bytes[2..])
    } else if looks_like_utf16_be(bytes) {
        (Utf16Endianness::Big, bytes)
    } else if looks_like_utf16_le(bytes) {
        (Utf16Endianness::Little, bytes)
    } else {
        return None;
    };

    let chunks = raw.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return None;
    }

    let words = chunks
        .map(|chunk| match endianness {
            Utf16Endianness::Big => u16::from_be_bytes([chunk[0], chunk[1]]),
            Utf16Endianness::Little => u16::from_le_bytes([chunk[0], chunk[1]]),
        })
        .collect::<Vec<_>>();

    String::from_utf16(&words)
        .ok()
        .map(|value| value.trim_end_matches('\0').to_string())
}

fn looks_like_utf16_be(bytes: &[u8]) -> bool {
    looks_like_utf16_ascii(bytes, true)
}

fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    looks_like_utf16_ascii(bytes, false)
}

fn looks_like_utf16_ascii(bytes: &[u8], big_endian: bool) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }

    let mut pairs = 0usize;
    let mut ascii_pairs = 0usize;
    for chunk in bytes.chunks_exact(2) {
        pairs += 1;
        let (zero, ascii) = if big_endian {
            (chunk[0], chunk[1])
        } else {
            (chunk[1], chunk[0])
        };
        if zero == 0 && ascii.is_ascii() && !ascii.is_ascii_control() {
            ascii_pairs += 1;
        }
    }

    ascii_pairs > 0 && ascii_pairs * 2 >= pairs
}

#[derive(Clone, Copy)]
enum Utf16Endianness {
    Big,
    Little,
}

fn first_engine_string(root: &EngineValue, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| match engine_lookup(root, path) {
            Some(EngineValue::String(value)) => Some(value.clone()),
            Some(EngineValue::Name(value)) => Some(value.clone()),
            _ => None,
        })
}

fn first_engine_array<'a>(root: &'a EngineValue, paths: &[&[&str]]) -> Option<&'a [EngineValue]> {
    paths
        .iter()
        .find_map(|path| match engine_lookup(root, path) {
            Some(EngineValue::Array(values)) => Some(values.as_slice()),
            _ => None,
        })
}

fn engine_lookup<'a>(value: &'a EngineValue, path: &[&str]) -> Option<&'a EngineValue> {
    let mut current = value;
    for segment in path {
        current = match current {
            EngineValue::Dict(map) => map.get(*segment)?,
            EngineValue::Array(list) => list.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn component_to_f64(value: &EngineValue) -> Option<f64> {
    match value {
        EngineValue::Number(value) => Some(*value),
        _ => None,
    }
}

fn component_to_usize(value: &EngineValue) -> Option<usize> {
    component_to_f64(value).and_then(|value| {
        if value.is_finite() && value >= 0.0 {
            Some(value as usize)
        } else {
            None
        }
    })
}

fn read_descriptor(cursor: &mut SliceCursor<'_>) -> std::result::Result<Descriptor, String> {
    let name_len = read_u32(cursor)? as u64;
    cursor.skip(name_len * 2)?;
    let class_id = read_pascal_id(cursor)?;
    let item_count = read_u32(cursor)?;
    let mut items = Vec::with_capacity(item_count as usize);
    for _ in 0..item_count {
        let key = read_pascal_id(cursor)?;
        let value = read_descriptor_value(cursor)?;
        items.push((key, value));
    }
    Ok(Descriptor { class_id, items })
}

fn read_descriptor_value(
    cursor: &mut SliceCursor<'_>,
) -> std::result::Result<DescriptorValue, String> {
    let kind = read_b4_string(cursor)?;
    match kind.as_str() {
        "long" => Ok(DescriptorValue::Integer(read_i32(cursor)?)),
        "doub" => Ok(DescriptorValue::Double(read_f64(cursor)?)),
        "UntF" => Ok(DescriptorValue::UnitFloat {
            unit: read_b4_string(cursor)?,
            value: read_f64(cursor)?,
        }),
        "bool" => Ok(DescriptorValue::Boolean(read_u8(cursor)? != 0)),
        "TEXT" => {
            let len = read_u32(cursor)? as usize;
            let mut words = Vec::with_capacity(len);
            for _ in 0..len {
                words.push(read_u16(cursor)?);
            }
            Ok(DescriptorValue::Text(
                String::from_utf16_lossy(&words)
                    .trim_end_matches('\0')
                    .to_string(),
            ))
        }
        "tdta" => {
            let len = read_u32(cursor)? as usize;
            let mut bytes = vec![0; len];
            cursor.read_exact(&mut bytes)?;
            Ok(DescriptorValue::RawData(bytes))
        }
        "Objc" => Ok(DescriptorValue::Object(Box::new(read_descriptor(cursor)?))),
        "enum" => Ok(DescriptorValue::Enum {
            kind: read_pascal_id(cursor)?,
            value: read_pascal_id(cursor)?,
        }),
        "VlLs" => {
            let count = read_u32(cursor)?;
            let mut values = Vec::with_capacity(count as usize);
            for _ in 0..count {
                values.push(read_descriptor_value(cursor)?);
            }
            Ok(DescriptorValue::List(values))
        }
        other => Err(format!("unsupported descriptor value type '{other}'")),
    }
}

fn read_pascal_id(cursor: &mut SliceCursor<'_>) -> std::result::Result<String, String> {
    let mut len = read_u32(cursor)?;
    if len == 0 {
        len = 4;
    }
    let mut bytes = vec![0; len as usize];
    cursor.read_exact(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn normalize_text_content(content: &str) -> String {
    content.replace('\u{0003}', "\n").replace('\r', "\n")
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn percent_to_unit(value: f32) -> f32 {
    (value / 100.0).clamp(0.0, 1.0)
}

fn clamp_color(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn scale_engine_color(value: f64) -> u8 {
    if value <= 1.0 {
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    } else {
        value.round().clamp(0.0, 255.0) as u8
    }
}

fn map_stroke_position(value: String) -> String {
    match value.as_str() {
        "OutF" | "outside" => "outside".to_string(),
        "InsF" | "inside" => "inside".to_string(),
        _ => "center".to_string(),
    }
}

fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn read_u8(cursor: &mut SliceCursor<'_>) -> std::result::Result<u8, String> {
    let mut buf = [0; 1];
    cursor.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16(cursor: &mut SliceCursor<'_>) -> std::result::Result<u16, String> {
    let mut buf = [0; 2];
    cursor.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

fn read_u32(cursor: &mut SliceCursor<'_>) -> std::result::Result<u32, String> {
    let mut buf = [0; 4];
    cursor.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

fn read_i32(cursor: &mut SliceCursor<'_>) -> std::result::Result<i32, String> {
    let mut buf = [0; 4];
    cursor.read_exact(&mut buf)?;
    Ok(i32::from_be_bytes(buf))
}

fn read_f64(cursor: &mut SliceCursor<'_>) -> std::result::Result<f64, String> {
    let mut buf = [0; 8];
    cursor.read_exact(&mut buf)?;
    Ok(f64::from_be_bytes(buf))
}

fn read_b4(cursor: &mut SliceCursor<'_>) -> std::result::Result<[u8; 4], String> {
    let mut buf = [0; 4];
    cursor.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_b4_string(cursor: &mut SliceCursor<'_>) -> std::result::Result<String, String> {
    Ok(String::from_utf8_lossy(&read_b4(cursor)?).to_string())
}

