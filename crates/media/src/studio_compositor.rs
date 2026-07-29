//! Deterministic CPU reference compositor shared by Studio preview and native
//! export verification.

use thiserror::Error;

use crate::{
    BackgroundStyle, CompositeStyle, LayoutPreset, NativeStudioPreviewFrame, PixelFormat,
    StudioCursorObservation, StudioCursorTimeline,
};

const MAX_STUDIO_COMPOSITE_PIXELS: u64 = 33_177_600;
const MAX_REFERENCE_BLUR_RADIUS: usize = 32;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum StudioCompositeError {
    #[error("Studio compositor input is invalid")]
    InvalidInput,
    #[error("Studio compositor requires a camera frame for this layout")]
    MissingCamera,
    #[error("Studio compositor exceeded its resource bound")]
    ResourceLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Composes one exact preview/export reference frame.
///
/// Cursor coordinates are normalized to the original screen frame and are
/// transformed with the screen surface for side-by-side output. Camera bubble
/// pixels are applied last, matching their higher visual z-order.
pub fn compose_studio_frame(
    screen: &NativeStudioPreviewFrame,
    camera: Option<&NativeStudioPreviewFrame>,
    cursor: Option<&StudioCursorTimeline>,
    source_time_ns: u64,
    style: CompositeStyle,
) -> Result<NativeStudioPreviewFrame, StudioCompositeError> {
    validate_frame(screen)?;
    if let Some(camera) = camera {
        validate_frame(camera)?;
    }
    let width = screen.width;
    let height = screen.height;
    let bytes = frame_bytes(width, height)?;
    let mut canvas = match style.background {
        BackgroundStyle::Transparent => vec![0; bytes],
        BackgroundStyle::SolidRgb { red, green, blue } => {
            solid_frame(width, height, [red, green, blue])?
        }
        BackgroundStyle::Blur { radius_milli } => {
            let full = PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            };
            let mut background = vec![0; bytes];
            blit_scaled(&mut background, width, height, screen, full, None)?;
            let radius = usize::from(radius_milli)
                .div_ceil(1_000)
                .clamp(1, MAX_REFERENCE_BLUR_RADIUS);
            blur_rgb(&background, width, height, radius)?
        }
    };

    let full = PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let (screen_rect, camera_rect) = match style.layout {
        LayoutPreset::ScreenOnly => (Some(full), None),
        LayoutPreset::CameraBubble => {
            let camera_rect = camera
                .map(|_| normalized_rect(width, height, style.camera.rect))
                .transpose()?;
            (Some(full), camera_rect)
        }
        LayoutPreset::SideBySide => {
            if camera.is_none() {
                return Err(StudioCompositeError::MissingCamera);
            }
            let left_width = width / 2;
            if left_width == 0 || width - left_width == 0 {
                return Err(StudioCompositeError::InvalidInput);
            }
            (
                Some(PixelRect {
                    x: 0,
                    y: 0,
                    width: left_width,
                    height,
                }),
                Some(PixelRect {
                    x: left_width,
                    y: 0,
                    width: width - left_width,
                    height,
                }),
            )
        }
        LayoutPreset::CameraFull => {
            if camera.is_none() {
                return Err(StudioCompositeError::MissingCamera);
            }
            (None, Some(full))
        }
    };

    if let Some(rect) = screen_rect {
        blit_scaled(&mut canvas, width, height, screen, rect, None)?;
        if !style.cursor.hidden
            && let Some(timeline) = cursor
            && let Some(observation) = timeline.observation_at_ns(source_time_ns)
        {
            overlay_cursor(
                &mut canvas,
                width,
                height,
                rect,
                timeline,
                observation,
                style.cursor.scale_milli,
            )?;
        }
    }
    if let (Some(camera), Some(rect)) = (camera, camera_rect) {
        let radius_milli = if style.layout == LayoutPreset::CameraBubble {
            style.camera.corner_radius_milli
        } else {
            0
        };
        blit_scaled(&mut canvas, width, height, camera, rect, Some(radius_milli))?;
    }

    Ok(NativeStudioPreviewFrame {
        width,
        height,
        pts_ns: screen.pts_ns,
        rgb: canvas,
    })
}

fn validate_frame(frame: &NativeStudioPreviewFrame) -> Result<(), StudioCompositeError> {
    if frame.width == 0
        || frame.height == 0
        || frame.rgb.len() != frame_bytes(frame.width, frame.height)?
    {
        return Err(StudioCompositeError::InvalidInput);
    }
    Ok(())
}

fn frame_bytes(width: u32, height: u32) -> Result<usize, StudioCompositeError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_STUDIO_COMPOSITE_PIXELS)
        .ok_or(StudioCompositeError::ResourceLimit)?;
    usize::try_from(
        pixels
            .checked_mul(3)
            .ok_or(StudioCompositeError::ResourceLimit)?,
    )
    .map_err(|_| StudioCompositeError::ResourceLimit)
}

fn solid_frame(width: u32, height: u32, color: [u8; 3]) -> Result<Vec<u8>, StudioCompositeError> {
    let mut frame = Vec::with_capacity(frame_bytes(width, height)?);
    for _ in 0..u64::from(width) * u64::from(height) {
        frame.extend_from_slice(&color);
    }
    Ok(frame)
}

fn normalized_rect(
    width: u32,
    height: u32,
    rect: crate::NormalizedRect,
) -> Result<PixelRect, StudioCompositeError> {
    rect.validate()
        .map_err(|_| StudioCompositeError::InvalidInput)?;
    let x = scale_millionths(width, rect.x_millionths)?;
    let y = scale_millionths(height, rect.y_millionths)?;
    let right = scale_millionths(
        width,
        rect.x_millionths
            .checked_add(rect.width_millionths)
            .ok_or(StudioCompositeError::ResourceLimit)?,
    )?;
    let bottom = scale_millionths(
        height,
        rect.y_millionths
            .checked_add(rect.height_millionths)
            .ok_or(StudioCompositeError::ResourceLimit)?,
    )?;
    Ok(PixelRect {
        x,
        y,
        width: right
            .checked_sub(x)
            .filter(|value| *value > 0)
            .ok_or(StudioCompositeError::InvalidInput)?,
        height: bottom
            .checked_sub(y)
            .filter(|value| *value > 0)
            .ok_or(StudioCompositeError::InvalidInput)?,
    })
}

fn scale_millionths(value: u32, millionths: u32) -> Result<u32, StudioCompositeError> {
    u32::try_from(
        u64::from(value)
            .checked_mul(u64::from(millionths))
            .ok_or(StudioCompositeError::ResourceLimit)?
            / 1_000_000,
    )
    .map_err(|_| StudioCompositeError::ResourceLimit)
}

fn blit_scaled(
    canvas: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: &NativeStudioPreviewFrame,
    target: PixelRect,
    rounded_radius_milli: Option<u16>,
) -> Result<(), StudioCompositeError> {
    let right = target
        .x
        .checked_add(target.width)
        .filter(|right| *right <= canvas_width)
        .ok_or(StudioCompositeError::InvalidInput)?;
    let bottom = target
        .y
        .checked_add(target.height)
        .filter(|bottom| *bottom <= canvas_height)
        .ok_or(StudioCompositeError::InvalidInput)?;
    let radius = rounded_radius_milli.map_or(0, |milli| {
        u64::from(target.width.min(target.height))
            .saturating_mul(u64::from(milli))
            .saturating_div(2_000) as u32
    });
    for y in target.y..bottom {
        for x in target.x..right {
            let local_x = x - target.x;
            let local_y = y - target.y;
            if radius > 0 && outside_rounded_rect(local_x, local_y, target, radius) {
                continue;
            }
            let source_x = u32::try_from(
                u64::from(local_x) * u64::from(source.width) / u64::from(target.width),
            )
            .map_err(|_| StudioCompositeError::ResourceLimit)?
            .min(source.width - 1);
            let source_y = u32::try_from(
                u64::from(local_y) * u64::from(source.height) / u64::from(target.height),
            )
            .map_err(|_| StudioCompositeError::ResourceLimit)?
            .min(source.height - 1);
            let source_offset = rgb_offset(source.width, source_x, source_y)?;
            let target_offset = rgb_offset(canvas_width, x, y)?;
            canvas[target_offset..target_offset + 3]
                .copy_from_slice(&source.rgb[source_offset..source_offset + 3]);
        }
    }
    Ok(())
}

fn outside_rounded_rect(x: u32, y: u32, rect: PixelRect, radius: u32) -> bool {
    let right_distance = rect.width - 1 - x;
    let bottom_distance = rect.height - 1 - y;
    let corner = if x < radius && y < radius {
        Some((radius - x, radius - y))
    } else if right_distance < radius && y < radius {
        Some((radius - right_distance, radius - y))
    } else if x < radius && bottom_distance < radius {
        Some((radius - x, radius - bottom_distance))
    } else if right_distance < radius && bottom_distance < radius {
        Some((radius - right_distance, radius - bottom_distance))
    } else {
        None
    };
    corner.is_some_and(|(dx, dy)| {
        u64::from(dx) * u64::from(dx) + u64::from(dy) * u64::from(dy)
            > u64::from(radius) * u64::from(radius)
    })
}

#[allow(clippy::too_many_arguments)]
fn overlay_cursor(
    canvas: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    screen_rect: PixelRect,
    timeline: &StudioCursorTimeline,
    observation: &StudioCursorObservation,
    scale_milli: u16,
) -> Result<(), StudioCompositeError> {
    let Some((frame_x, frame_y)) = observation.frame_position() else {
        return Ok(());
    };
    let Some(revision) = observation.image_revision() else {
        return Ok(());
    };
    let image = timeline
        .image(revision)
        .ok_or(StudioCompositeError::InvalidInput)?;
    let (timeline_width, timeline_height) = timeline.frame_dimensions();
    if timeline_width == 0 || timeline_height == 0 || !(100..=4_000).contains(&scale_milli) {
        return Err(StudioCompositeError::InvalidInput);
    }
    let image_width = scaled_cursor_dimension(
        u32::from(image.dimensions().0),
        screen_rect.width,
        timeline_width,
        scale_milli,
    )?;
    let image_height = scaled_cursor_dimension(
        u32::from(image.dimensions().1),
        screen_rect.height,
        timeline_height,
        scale_milli,
    )?;
    let hotspot_x = scaled_cursor_dimension(
        u32::from(image.hotspot().0),
        screen_rect.width,
        timeline_width,
        scale_milli,
    )?;
    let hotspot_y = scaled_cursor_dimension(
        u32::from(image.hotspot().1),
        screen_rect.height,
        timeline_height,
        scale_milli,
    )?;
    let anchor_x = screen_rect.x
        + u32::try_from(
            u64::from(frame_x) * u64::from(screen_rect.width) / u64::from(timeline_width),
        )
        .map_err(|_| StudioCompositeError::ResourceLimit)?;
    let anchor_y = screen_rect.y
        + u32::try_from(
            u64::from(frame_y) * u64::from(screen_rect.height) / u64::from(timeline_height),
        )
        .map_err(|_| StudioCompositeError::ResourceLimit)?;
    let origin_x = i64::from(anchor_x) - i64::from(hotspot_x);
    let origin_y = i64::from(anchor_y) - i64::from(hotspot_y);
    for target_y in 0..image_height {
        for target_x in 0..image_width {
            let canvas_x = origin_x + i64::from(target_x);
            let canvas_y = origin_y + i64::from(target_y);
            if canvas_x < 0
                || canvas_y < 0
                || canvas_x >= i64::from(canvas_width)
                || canvas_y >= i64::from(canvas_height)
            {
                continue;
            }
            let source_x = u32::try_from(
                u64::from(target_x) * u64::from(image.dimensions().0) / u64::from(image_width),
            )
            .map_err(|_| StudioCompositeError::ResourceLimit)?
            .min(u32::from(image.dimensions().0) - 1);
            let source_y = u32::try_from(
                u64::from(target_y) * u64::from(image.dimensions().1) / u64::from(image_height),
            )
            .map_err(|_| StudioCompositeError::ResourceLimit)?
            .min(u32::from(image.dimensions().1) - 1);
            let source_offset = usize::try_from(
                (u64::from(source_y) * u64::from(image.dimensions().0) + u64::from(source_x)) * 4,
            )
            .map_err(|_| StudioCompositeError::ResourceLimit)?;
            let target_offset = rgb_offset(
                canvas_width,
                u32::try_from(canvas_x).map_err(|_| StudioCompositeError::ResourceLimit)?,
                u32::try_from(canvas_y).map_err(|_| StudioCompositeError::ResourceLimit)?,
            )?;
            blend_cursor_pixel(
                &mut canvas[target_offset..target_offset + 3],
                &image.pixels()[source_offset..source_offset + 4],
                image.pixel_format(),
            );
        }
    }
    if observation.primary_click() || observation.secondary_click() {
        draw_click_marker(
            canvas,
            canvas_width,
            canvas_height,
            anchor_x,
            anchor_y,
            observation.primary_click(),
        )?;
    }
    Ok(())
}

fn scaled_cursor_dimension(
    value: u32,
    target: u32,
    source: u32,
    scale_milli: u16,
) -> Result<u32, StudioCompositeError> {
    if value == 0 {
        return Ok(0);
    }
    u32::try_from(
        u64::from(value)
            .checked_mul(u64::from(target))
            .and_then(|value| value.checked_mul(u64::from(scale_milli)))
            .ok_or(StudioCompositeError::ResourceLimit)?
            .div_ceil(
                u64::from(source)
                    .checked_mul(1_000)
                    .ok_or(StudioCompositeError::ResourceLimit)?,
            ),
    )
    .map(|value| value.max(1))
    .map_err(|_| StudioCompositeError::ResourceLimit)
}

fn blend_cursor_pixel(target: &mut [u8], source: &[u8], format: PixelFormat) {
    let (red, green, blue, alpha) = match format {
        PixelFormat::Rgba8 => (source[0], source[1], source[2], source[3]),
        PixelFormat::Bgra8 => (source[2], source[1], source[0], source[3]),
        PixelFormat::Nv12 | PixelFormat::I420 => return,
    };
    let alpha = u16::from(alpha);
    let inverse = 255 - alpha;
    target[0] = ((u16::from(red) * alpha + u16::from(target[0]) * inverse + 127) / 255) as u8;
    target[1] = ((u16::from(green) * alpha + u16::from(target[1]) * inverse + 127) / 255) as u8;
    target[2] = ((u16::from(blue) * alpha + u16::from(target[2]) * inverse + 127) / 255) as u8;
}

fn draw_click_marker(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    center_x: u32,
    center_y: u32,
    primary: bool,
) -> Result<(), StudioCompositeError> {
    let color = if primary {
        [255, 196, 0]
    } else {
        [0, 196, 255]
    };
    let radius = 5_i64;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let distance = dx * dx + dy * dy;
            if !(16..=25).contains(&distance) {
                continue;
            }
            let x = i64::from(center_x) + dx;
            let y = i64::from(center_y) + dy;
            if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
                continue;
            }
            let offset = rgb_offset(
                width,
                u32::try_from(x).map_err(|_| StudioCompositeError::ResourceLimit)?,
                u32::try_from(y).map_err(|_| StudioCompositeError::ResourceLimit)?,
            )?;
            canvas[offset..offset + 3].copy_from_slice(&color);
        }
    }
    Ok(())
}

fn rgb_offset(width: u32, x: u32, y: u32) -> Result<usize, StudioCompositeError> {
    usize::try_from(
        (u64::from(y)
            .checked_mul(u64::from(width))
            .and_then(|value| value.checked_add(u64::from(x)))
            .ok_or(StudioCompositeError::ResourceLimit)?)
        .checked_mul(3)
        .ok_or(StudioCompositeError::ResourceLimit)?,
    )
    .map_err(|_| StudioCompositeError::ResourceLimit)
}

fn blur_rgb(
    input: &[u8],
    width: u32,
    height: u32,
    radius: usize,
) -> Result<Vec<u8>, StudioCompositeError> {
    let width = usize::try_from(width).map_err(|_| StudioCompositeError::ResourceLimit)?;
    let height = usize::try_from(height).map_err(|_| StudioCompositeError::ResourceLimit)?;
    let mut horizontal = vec![0; input.len()];
    let mut output = vec![0; input.len()];
    for y in 0..height {
        for channel in 0..3 {
            let mut sum = 0_u64;
            for x in 0..width {
                if x == 0 {
                    for sample_x in 0..=radius.min(width - 1) {
                        sum += u64::from(input[(y * width + sample_x) * 3 + channel]);
                    }
                } else {
                    let add_x = x.saturating_add(radius).min(width - 1);
                    if add_x > (x - 1).saturating_add(radius).min(width - 1) {
                        sum += u64::from(input[(y * width + add_x) * 3 + channel]);
                    }
                    if x > radius {
                        sum -= u64::from(input[(y * width + x - radius - 1) * 3 + channel]);
                    }
                }
                let start = x.saturating_sub(radius);
                let end = x.saturating_add(radius).min(width - 1);
                horizontal[(y * width + x) * 3 + channel] =
                    u8::try_from(sum / u64::try_from(end - start + 1).unwrap_or(1)).unwrap_or(255);
            }
        }
    }
    for x in 0..width {
        for channel in 0..3 {
            let mut sum = 0_u64;
            for y in 0..height {
                if y == 0 {
                    for sample_y in 0..=radius.min(height - 1) {
                        sum += u64::from(horizontal[(sample_y * width + x) * 3 + channel]);
                    }
                } else {
                    let add_y = y.saturating_add(radius).min(height - 1);
                    if add_y > (y - 1).saturating_add(radius).min(height - 1) {
                        sum += u64::from(horizontal[(add_y * width + x) * 3 + channel]);
                    }
                    if y > radius {
                        sum -= u64::from(horizontal[((y - radius - 1) * width + x) * 3 + channel]);
                    }
                }
                let start = y.saturating_sub(radius);
                let end = y.saturating_add(radius).min(height - 1);
                output[(y * width + x) * 3 + channel] =
                    u8::try_from(sum / u64::try_from(end - start + 1).unwrap_or(1)).unwrap_or(255);
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CameraStyle, CursorStyle, FrameTimestamp, NormalizedRect, StudioCursorImage,
        StudioCursorObservation,
    };

    fn frame(width: u32, height: u32, color: [u8; 3]) -> NativeStudioPreviewFrame {
        NativeStudioPreviewFrame {
            width,
            height,
            pts_ns: 42,
            rgb: solid_frame(width, height, color).expect("frame"),
        }
    }

    fn cursor() -> StudioCursorTimeline {
        let image = StudioCursorImage::new(
            1,
            2,
            2,
            0,
            0,
            PixelFormat::Rgba8,
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
        )
        .expect("image");
        StudioCursorTimeline::new(
            8,
            4,
            vec![
                StudioCursorObservation::new(
                    1,
                    FrameTimestamp::new(0, 10).expect("timestamp"),
                    true,
                    2,
                    1,
                    Some(1),
                    false,
                    false,
                    Some(image),
                )
                .expect("observation"),
            ],
        )
        .expect("timeline")
    }

    fn style(layout: LayoutPreset) -> CompositeStyle {
        CompositeStyle {
            layout,
            camera: CameraStyle {
                rect: NormalizedRect {
                    x_millionths: 500_000,
                    y_millionths: 0,
                    width_millionths: 500_000,
                    height_millionths: 1_000_000,
                },
                corner_radius_milli: 0,
            },
            cursor: CursorStyle {
                scale_milli: 1_000,
                hidden: false,
            },
            background: BackgroundStyle::SolidRgb {
                red: 1,
                green: 2,
                blue: 3,
            },
            ..CompositeStyle::default()
        }
    }

    #[test]
    fn side_by_side_transforms_screen_cursor_and_camera_deterministically() {
        let screen = frame(8, 4, [0, 0, 255]);
        let camera = frame(8, 4, [0, 255, 0]);
        let output = compose_studio_frame(
            &screen,
            Some(&camera),
            Some(&cursor()),
            0,
            style(LayoutPreset::SideBySide),
        )
        .expect("composition");
        assert_eq!(
            &output.rgb[rgb_offset(8, 0, 0).expect("offset")..][..3],
            &[0, 0, 255]
        );
        assert_eq!(
            &output.rgb[rgb_offset(8, 1, 1).expect("offset")..][..3],
            &[255, 0, 0]
        );
        assert_eq!(
            &output.rgb[rgb_offset(8, 4, 1).expect("offset")..][..3],
            &[0, 255, 0]
        );
    }

    #[test]
    fn camera_layouts_require_camera_and_rounded_bubble_preserves_screen_corner() {
        let screen = frame(8, 4, [0, 0, 255]);
        assert_eq!(
            compose_studio_frame(&screen, None, None, 0, style(LayoutPreset::CameraFull)),
            Err(StudioCompositeError::MissingCamera)
        );
        let mut bubble = style(LayoutPreset::CameraBubble);
        bubble.camera.corner_radius_milli = 1_000;
        let output =
            compose_studio_frame(&screen, Some(&frame(8, 4, [0, 255, 0])), None, 0, bubble)
                .expect("bubble");
        assert_eq!(
            &output.rgb[rgb_offset(8, 4, 0).expect("offset")..][..3],
            &[0, 0, 255]
        );
        assert_eq!(
            &output.rgb[rgb_offset(8, 6, 2).expect("offset")..][..3],
            &[0, 255, 0]
        );
    }

    #[test]
    fn hidden_cursor_and_blur_background_are_bounded() {
        let screen = frame(8, 4, [10, 20, 30]);
        let mut hidden = style(LayoutPreset::ScreenOnly);
        hidden.cursor.hidden = true;
        hidden.background = BackgroundStyle::Blur {
            radius_milli: 60_000,
        };
        let output =
            compose_studio_frame(&screen, None, Some(&cursor()), 0, hidden).expect("composition");
        assert_eq!(output.rgb, screen.rgb);
    }
}
