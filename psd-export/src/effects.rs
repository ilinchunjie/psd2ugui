use std::collections::VecDeque;

use crate::manifest::{
    Bounds, ColorRgba, DropShadowEffect, ExportWarning, LayerEffects, StrokeEffect,
};
use crate::psd::{MaskBitmap, RgbaBitmap};

#[derive(Debug, Default)]
pub(crate) struct EffectBakeOutcome {
    pub image: Option<RgbaBitmap>,
    pub bounds: Option<Bounds>,
    pub baked: Vec<String>,
    pub warnings: Vec<ExportWarning>,
}

#[derive(Clone, Copy, Debug)]
struct StrokeConfig {
    radius: usize,
    color: ColorRgba,
    opacity: f32,
    position_approximated: bool,
    blend_mode_approximated: bool,
}

#[derive(Clone, Copy, Debug)]
struct ShadowConfig {
    blur_radius: usize,
    offset_x: i32,
    offset_y: i32,
    color: ColorRgba,
    opacity: f32,
    blend_mode_approximated: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EffectPadding {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

pub(crate) fn bake_layer_effects(
    layer_name: &str,
    bitmap: &RgbaBitmap,
    bounds: &Bounds,
    mask: Option<&MaskBitmap>,
    clip_to: Option<&str>,
    effects: &LayerEffects,
) -> EffectBakeOutcome {
    let stroke = resolve_stroke(effects.stroke.as_ref());
    let shadow = resolve_shadow(effects.drop_shadow.as_ref());
    if stroke.is_none() && shadow.is_none() {
        return EffectBakeOutcome::default();
    }

    let mut warnings = Vec::new();
    if mask.is_some() {
        warnings.push(ExportWarning::for_layer(
            "effects-bake-skipped-mask",
            "skipped effect baking because masked layers are not baked in phase one",
            layer_name,
        ));
        return EffectBakeOutcome {
            warnings,
            ..EffectBakeOutcome::default()
        };
    }

    if clip_to.is_some() {
        warnings.push(ExportWarning::for_layer(
            "effects-bake-skipped-clipping",
            "skipped effect baking because clipped layers are not baked in phase one",
            layer_name,
        ));
        return EffectBakeOutcome {
            warnings,
            ..EffectBakeOutcome::default()
        };
    }

    if let Some(stroke) = stroke {
        if stroke.position_approximated {
            warnings.push(ExportWarning::for_layer(
                "effects-bake-stroke-position-approx",
                "stroke position was approximated as an outside stroke during PNG baking",
                layer_name,
            ));
        }
        if stroke.blend_mode_approximated {
            warnings.push(ExportWarning::for_layer(
                "effects-bake-stroke-blend-mode-approx",
                "stroke blend mode was approximated as normal during PNG baking",
                layer_name,
            ));
        }
    }

    if let Some(shadow) = shadow {
        if shadow.blend_mode_approximated {
            warnings.push(ExportWarning::for_layer(
                "effects-bake-shadow-blend-mode-approx",
                "drop shadow blend mode was approximated as normal during PNG baking",
                layer_name,
            ));
        }
    }

    let padding = compute_padding(stroke, shadow);
    let final_width = bitmap.width + padding.left as u32 + padding.right as u32;
    let final_height = bitmap.height + padding.top as u32 + padding.bottom as u32;
    let source_origin_x = padding.left as usize;
    let source_origin_y = padding.top as usize;
    let pixels = final_width as usize * final_height as usize;
    let alpha_canvas = alpha_canvas(
        bitmap,
        final_width,
        final_height,
        source_origin_x,
        source_origin_y,
    );

    let mut composed = vec![0u8; pixels * 4];
    let mut baked = Vec::new();

    if let Some(shadow) = shadow {
        let shifted_alpha = shift_alpha_mask(
            &alpha_canvas,
            final_width,
            final_height,
            shadow.offset_x,
            shadow.offset_y,
        );
        let blurred_alpha = if shadow.blur_radius == 0 {
            shifted_alpha
        } else {
            box_blur_u8(
                &shifted_alpha,
                final_width as usize,
                final_height as usize,
                shadow.blur_radius,
            )
        };
        composite_solid_mask(
            &mut composed,
            &blurred_alpha,
            shadow.color,
            combined_opacity(shadow.color, shadow.opacity),
        );
        baked.push("drop_shadow".to_string());
    }

    if let Some(stroke) = stroke {
        let dilated_alpha = max_filter_u8(
            &alpha_canvas,
            final_width as usize,
            final_height as usize,
            stroke.radius,
        );
        let outer_alpha = dilated_alpha
            .iter()
            .zip(alpha_canvas.iter())
            .map(|(expanded, original)| expanded.saturating_sub(*original))
            .collect::<Vec<_>>();
        composite_solid_mask(
            &mut composed,
            &outer_alpha,
            stroke.color,
            combined_opacity(stroke.color, stroke.opacity),
        );
        baked.push("stroke".to_string());
    }

    composite_bitmap(
        &mut composed,
        final_width as usize,
        final_height as usize,
        bitmap,
        source_origin_x,
        source_origin_y,
    );

    EffectBakeOutcome {
        image: Some(RgbaBitmap {
            width: final_width,
            height: final_height,
            pixels: composed,
        }),
        bounds: Some(Bounds {
            x: bounds.x - padding.left,
            y: bounds.y - padding.top,
            width: final_width,
            height: final_height,
        }),
        baked,
        warnings,
    }
}

fn resolve_stroke(effect: Option<&StrokeEffect>) -> Option<StrokeConfig> {
    let effect = effect?;
    if !effect.enabled {
        return None;
    }

    let radius = effect.size.unwrap_or(1.0).ceil().max(0.0) as usize;
    if radius == 0 {
        return None;
    }

    Some(StrokeConfig {
        radius,
        color: effect.color.unwrap_or(default_shadow_color()),
        opacity: effect.opacity.unwrap_or(1.0).clamp(0.0, 1.0),
        position_approximated: !matches!(effect.position.as_deref(), None | Some("outside")),
        blend_mode_approximated: !is_normal_blend_mode(effect.blend_mode.as_deref()),
    })
}

fn resolve_shadow(effect: Option<&DropShadowEffect>) -> Option<ShadowConfig> {
    let effect = effect?;
    if !effect.enabled {
        return None;
    }

    let opacity = effect.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return None;
    }

    let distance = effect.distance.unwrap_or(0.0);
    let angle = effect.angle.unwrap_or(120.0);
    let radians = angle.to_radians();
    Some(ShadowConfig {
        blur_radius: effect.blur.unwrap_or(0.0).ceil().max(0.0) as usize,
        offset_x: (distance * radians.cos()).round() as i32,
        offset_y: (distance * radians.sin()).round() as i32,
        color: effect.color.unwrap_or(default_shadow_color()),
        opacity,
        blend_mode_approximated: !is_normal_blend_mode(effect.blend_mode.as_deref()),
    })
}

fn compute_padding(stroke: Option<StrokeConfig>, shadow: Option<ShadowConfig>) -> EffectPadding {
    let mut padding = EffectPadding::default();

    if let Some(stroke) = stroke {
        let radius = stroke.radius as i32;
        padding.left = padding.left.max(radius);
        padding.top = padding.top.max(radius);
        padding.right = padding.right.max(radius);
        padding.bottom = padding.bottom.max(radius);
    }

    if let Some(shadow) = shadow {
        let blur = shadow.blur_radius as i32;
        padding.left = padding.left.max((blur - shadow.offset_x).max(0));
        padding.top = padding.top.max((blur - shadow.offset_y).max(0));
        padding.right = padding.right.max((blur + shadow.offset_x).max(0));
        padding.bottom = padding.bottom.max((blur + shadow.offset_y).max(0));
    }

    padding
}

fn default_shadow_color() -> ColorRgba {
    ColorRgba {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    }
}

fn is_normal_blend_mode(mode: Option<&str>) -> bool {
    matches!(mode, None | Some("") | Some("norm") | Some("normal"))
}

fn combined_opacity(color: ColorRgba, opacity: f32) -> f32 {
    ((color.a as f32 / 255.0) * opacity).clamp(0.0, 1.0)
}

fn alpha_canvas(
    bitmap: &RgbaBitmap,
    canvas_width: u32,
    canvas_height: u32,
    origin_x: usize,
    origin_y: usize,
) -> Vec<u8> {
    let mut alpha = vec![0u8; canvas_width as usize * canvas_height as usize];
    for row in 0..bitmap.height as usize {
        for column in 0..bitmap.width as usize {
            let source_index = (row * bitmap.width as usize + column) * 4 + 3;
            let destination_index = (origin_y + row) * canvas_width as usize + origin_x + column;
            alpha[destination_index] = bitmap.pixels[source_index];
        }
    }
    alpha
}

fn shift_alpha_mask(mask: &[u8], width: u32, height: u32, offset_x: i32, offset_y: i32) -> Vec<u8> {
    let mut shifted = vec![0u8; mask.len()];
    let width = width as usize;
    let height = height as usize;
    for row in 0..height {
        for column in 0..width {
            let alpha = mask[row * width + column];
            if alpha == 0 {
                continue;
            }

            let destination_x = column as i32 + offset_x;
            let destination_y = row as i32 + offset_y;
            if destination_x < 0
                || destination_y < 0
                || destination_x >= width as i32
                || destination_y >= height as i32
            {
                continue;
            }

            let destination_index = destination_y as usize * width + destination_x as usize;
            shifted[destination_index] = shifted[destination_index].max(alpha);
        }
    }
    shifted
}

fn box_blur_u8(input: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 || input.is_empty() {
        return input.to_vec();
    }

    let mut horizontal = vec![0u8; input.len()];
    for row in 0..height {
        let start = row * width;
        let blurred = box_blur_line(&input[start..start + width], radius);
        horizontal[start..start + width].copy_from_slice(&blurred);
    }

    let mut output = vec![0u8; input.len()];
    let mut column = vec![0u8; height];
    for x in 0..width {
        for y in 0..height {
            column[y] = horizontal[y * width + x];
        }
        let blurred = box_blur_line(&column, radius);
        for y in 0..height {
            output[y * width + x] = blurred[y];
        }
    }
    output
}

fn box_blur_line(line: &[u8], radius: usize) -> Vec<u8> {
    let mut prefix = vec![0u32; line.len() + 1];
    for (index, value) in line.iter().enumerate() {
        prefix[index + 1] = prefix[index] + *value as u32;
    }

    let mut output = vec![0u8; line.len()];
    for index in 0..line.len() {
        let start = index.saturating_sub(radius);
        let end = (index + radius).min(line.len().saturating_sub(1));
        let total = prefix[end + 1] - prefix[start];
        let samples = (end - start + 1) as u32;
        output[index] = (total / samples) as u8;
    }
    output
}

fn max_filter_u8(input: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 || input.is_empty() {
        return input.to_vec();
    }

    let mut horizontal = vec![0u8; input.len()];
    for row in 0..height {
        let start = row * width;
        let filtered = max_filter_line(&input[start..start + width], radius);
        horizontal[start..start + width].copy_from_slice(&filtered);
    }

    let mut output = vec![0u8; input.len()];
    let mut column = vec![0u8; height];
    for x in 0..width {
        for y in 0..height {
            column[y] = horizontal[y * width + x];
        }
        let filtered = max_filter_line(&column, radius);
        for y in 0..height {
            output[y * width + x] = filtered[y];
        }
    }
    output
}

fn max_filter_line(line: &[u8], radius: usize) -> Vec<u8> {
    if line.is_empty() {
        return Vec::new();
    }

    let mut output = vec![0u8; line.len()];
    let mut deque = VecDeque::<usize>::new();
    let mut next_index = 0usize;

    for center in 0..line.len() {
        let end = (center + radius).min(line.len() - 1);
        while next_index <= end {
            while let Some(&back) = deque.back() {
                if line[back] <= line[next_index] {
                    deque.pop_back();
                } else {
                    break;
                }
            }
            deque.push_back(next_index);
            next_index += 1;
        }

        let start = center.saturating_sub(radius);
        while let Some(&front) = deque.front() {
            if front < start {
                deque.pop_front();
            } else {
                break;
            }
        }

        output[center] = line[*deque.front().expect("deque should not be empty")];
    }

    output
}

fn composite_solid_mask(destination: &mut [u8], mask: &[u8], color: ColorRgba, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }

    for (index, alpha) in mask.iter().enumerate() {
        if *alpha == 0 {
            continue;
        }

        let source_alpha = scale_alpha(*alpha, opacity);
        if source_alpha == 0 {
            continue;
        }

        composite_pixel(
            &mut destination[index * 4..index * 4 + 4],
            [color.r, color.g, color.b, source_alpha],
        );
    }
}

fn composite_bitmap(
    destination: &mut [u8],
    destination_width: usize,
    destination_height: usize,
    bitmap: &RgbaBitmap,
    origin_x: usize,
    origin_y: usize,
) {
    for row in 0..bitmap.height as usize {
        let destination_row = origin_y + row;
        if destination_row >= destination_height {
            continue;
        }

        for column in 0..bitmap.width as usize {
            let destination_column = origin_x + column;
            if destination_column >= destination_width {
                continue;
            }

            let source_index = (row * bitmap.width as usize + column) * 4;
            let destination_index = (destination_row * destination_width + destination_column) * 4;
            composite_pixel(
                &mut destination[destination_index..destination_index + 4],
                [
                    bitmap.pixels[source_index],
                    bitmap.pixels[source_index + 1],
                    bitmap.pixels[source_index + 2],
                    bitmap.pixels[source_index + 3],
                ],
            );
        }
    }
}

fn composite_pixel(destination: &mut [u8], source: [u8; 4]) {
    let source_alpha = source[3] as f32 / 255.0;
    if source_alpha <= 0.0 {
        return;
    }

    let destination_alpha = destination[3] as f32 / 255.0;
    let out_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let source_weight = source_alpha;
    let destination_weight = destination_alpha * (1.0 - source_alpha);

    for channel in 0..3 {
        let blended = source[channel] as f32 * source_weight
            + destination[channel] as f32 * destination_weight;
        destination[channel] = if out_alpha > 0.0 {
            (blended / out_alpha).round().clamp(0.0, 255.0) as u8
        } else {
            0
        };
    }
    destination[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn scale_alpha(alpha: u8, opacity: f32) -> u8 {
    (alpha as f32 * opacity).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{DropShadowEffect, LayerEffects, StrokeEffect};

    #[test]
    fn computes_symmetric_padding_for_outer_stroke() {
        let padding = compute_padding(
            Some(StrokeConfig {
                radius: 3,
                color: default_shadow_color(),
                opacity: 1.0,
                position_approximated: false,
                blend_mode_approximated: false,
            }),
            None,
        );

        assert_eq!(
            padding,
            EffectPadding {
                left: 3,
                top: 3,
                right: 3,
                bottom: 3,
            }
        );
    }

    #[test]
    fn computes_padding_for_negative_shadow_offset() {
        let padding = compute_padding(
            None,
            Some(ShadowConfig {
                blur_radius: 2,
                offset_x: -4,
                offset_y: 3,
                color: default_shadow_color(),
                opacity: 1.0,
                blend_mode_approximated: false,
            }),
        );

        assert_eq!(
            padding,
            EffectPadding {
                left: 6,
                top: 0,
                right: 0,
                bottom: 5,
            }
        );
    }

    #[test]
    fn combines_padding_from_stroke_and_shadow() {
        let padding = compute_padding(
            Some(StrokeConfig {
                radius: 2,
                color: default_shadow_color(),
                opacity: 1.0,
                position_approximated: false,
                blend_mode_approximated: false,
            }),
            Some(ShadowConfig {
                blur_radius: 1,
                offset_x: 5,
                offset_y: -3,
                color: default_shadow_color(),
                opacity: 1.0,
                blend_mode_approximated: false,
            }),
        );

        assert_eq!(
            padding,
            EffectPadding {
                left: 2,
                top: 4,
                right: 6,
                bottom: 2,
            }
        );
    }

    #[test]
    fn bakes_stroke_and_shadow_into_expected_pixels() {
        let bitmap = opaque_bitmap(1, 1, [255, 255, 255, 255]);
        let bounds = Bounds {
            x: 10,
            y: 20,
            width: 1,
            height: 1,
        };
        let effects = LayerEffects {
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
            drop_shadow: Some(DropShadowEffect {
                color: Some(default_shadow_color()),
                opacity: Some(1.0),
                blur: Some(0.0),
                distance: Some(2.0),
                angle: Some(0.0),
                blend_mode: None,
                enabled: true,
            }),
            ..LayerEffects::default()
        };

        let outcome = bake_layer_effects("Badge", &bitmap, &bounds, None, None, &effects);
        let image = outcome.image.expect("baked bitmap should exist");
        let baked_bounds = outcome.bounds.expect("bounds should expand");

        assert_eq!(baked_bounds.x, 9);
        assert_eq!(baked_bounds.y, 19);
        assert_eq!(baked_bounds.width, 4);
        assert_eq!(baked_bounds.height, 3);
        assert_eq!(
            outcome.baked,
            vec!["drop_shadow".to_string(), "stroke".to_string()]
        );
        assert_eq!(pixel_at(&image, 1, 1), [255, 255, 255, 255]);
        assert_eq!(pixel_at(&image, 0, 1), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&image, 3, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn expands_bounds_for_negative_shadow_offsets() {
        let bitmap = opaque_bitmap(1, 1, [255, 255, 255, 255]);
        let bounds = Bounds {
            x: 10,
            y: 20,
            width: 1,
            height: 1,
        };
        let effects = LayerEffects {
            drop_shadow: Some(DropShadowEffect {
                color: Some(default_shadow_color()),
                opacity: Some(1.0),
                blur: Some(2.0),
                distance: Some(3.0),
                angle: Some(180.0),
                blend_mode: None,
                enabled: true,
            }),
            ..LayerEffects::default()
        };

        let outcome = bake_layer_effects("Card", &bitmap, &bounds, None, None, &effects);
        let baked_bounds = outcome.bounds.expect("bounds should expand");

        assert_eq!(baked_bounds.x, 5);
        assert_eq!(baked_bounds.y, 18);
        assert_eq!(baked_bounds.width, 6);
        assert_eq!(baked_bounds.height, 5);
    }

    #[test]
    fn skips_baking_for_masked_layers() {
        let bitmap = opaque_bitmap(1, 1, [255, 255, 255, 255]);
        let bounds = Bounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let effects = LayerEffects {
            stroke: Some(StrokeEffect {
                color: Some(default_shadow_color()),
                opacity: Some(1.0),
                size: Some(2.0),
                position: Some("inside".to_string()),
                blend_mode: Some("multiply".to_string()),
                enabled: true,
            }),
            ..LayerEffects::default()
        };
        let mask = MaskBitmap {
            bounds: bounds.clone(),
            default_color: 0,
            relative: false,
            disabled: false,
            invert: false,
            pixels: vec![255],
        };

        let outcome = bake_layer_effects("Masked", &bitmap, &bounds, Some(&mask), None, &effects);

        assert!(outcome.image.is_none());
        assert!(outcome.bounds.is_none());
        assert!(outcome.baked.is_empty());
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].code, "effects-bake-skipped-mask");
    }

    fn opaque_bitmap(width: u32, height: u32, pixel: [u8; 4]) -> RgbaBitmap {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            pixels.extend_from_slice(&pixel);
        }
        RgbaBitmap {
            width,
            height,
            pixels,
        }
    }

    fn pixel_at(bitmap: &RgbaBitmap, x: usize, y: usize) -> [u8; 4] {
        let index = (y * bitmap.width as usize + x) * 4;
        [
            bitmap.pixels[index],
            bitmap.pixels[index + 1],
            bitmap.pixels[index + 2],
            bitmap.pixels[index + 3],
        ]
    }
}
