//! Bounded macOS cursor sampling for metadata capture.
//!
//! AppKit and Core Graphics bindings own their pointer-level implementation;
//! this module exposes only checked coordinates and exact RGBA allocations to
//! the normalized screen source.

use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSBitmapImageRep, NSColorSpace, NSCursor};
#[allow(deprecated)]
use objc2_core_graphics::{
    CGCursorIsVisible, CGEvent, CGEventSource, CGEventSourceStateID, CGMouseButton,
};
use ring::digest::{Context, SHA256};

use frame_media::{
    CursorCaptureMode, CursorImageDescriptor, CursorPolicy, PixelFormat, RawCursorObservation,
    RawCursorPosition, ScreenCursorImage, ScreenFrame, ScreenFrameEnvelope, ScreenSourceEvent,
    ScreenStreamStamp, ScreenTargetDescriptor, ScreenTargetKind, VideoFrameSpec,
    normalize_screen_cursor,
};

use crate::{MacOsCaptureError, MacOsCaptureFrame};

const MAX_CURSOR_DIMENSION: u16 = 256;
const BYTES_PER_PIXEL: usize = 4;
type CursorEvents = Vec<ScreenSourceEvent<Box<[u8]>, Box<[u8]>>>;

pub(super) struct CursorSampler {
    last_fingerprint: Option<[u8; 32]>,
    revision: u64,
}

pub(super) struct CursorMetadataSession {
    target: ScreenTargetDescriptor,
    output: VideoFrameSpec,
    policy: CursorPolicy,
    sampler: CursorSampler,
    pending_image: Option<CursorImage>,
    published_revision: Option<u64>,
}

pub(super) struct CursorSample {
    pub(super) visible: bool,
    pub(super) desktop_x: i32,
    pub(super) desktop_y: i32,
    pub(super) primary_click: bool,
    pub(super) secondary_click: bool,
    pub(super) image_revision: u64,
    pub(super) changed_image: Option<CursorImage>,
}

pub(super) struct CursorImage {
    pub(super) revision: u64,
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) hotspot_x: u16,
    pub(super) hotspot_y: u16,
    pub(super) rgba: Box<[u8]>,
}

impl CursorSampler {
    pub(super) const fn new() -> Self {
        Self {
            last_fingerprint: None,
            revision: 0,
        }
    }

    pub(super) fn sample(
        &mut self,
        include_image: bool,
    ) -> Result<CursorSample, MacOsCaptureError> {
        autoreleasepool(|_| self.sample_in_pool(include_image))
    }

    fn sample_in_pool(&mut self, include_image: bool) -> Result<CursorSample, MacOsCaptureError> {
        let event = CGEvent::new(None).ok_or(MacOsCaptureError::CursorMetadataUnavailable)?;
        let location = CGEvent::location(Some(&event));
        let desktop_x = checked_coordinate(location.x)?;
        let desktop_y = checked_coordinate(location.y)?;

        #[allow(deprecated)]
        let visible = CGCursorIsVisible();
        let primary_click =
            CGEventSource::button_state(CGEventSourceStateID::HIDSystemState, CGMouseButton::Left);
        let secondary_click =
            CGEventSource::button_state(CGEventSourceStateID::HIDSystemState, CGMouseButton::Right);
        if !visible || !include_image {
            return Ok(CursorSample {
                visible,
                desktop_x,
                desktop_y,
                primary_click: false,
                secondary_click: false,
                image_revision: if include_image { self.revision } else { 0 },
                changed_image: None,
            });
        }

        #[allow(deprecated)]
        let cursor = NSCursor::currentSystemCursor().unwrap_or_else(NSCursor::currentCursor);
        let native_image = cursor.image();
        let image_size = native_image.size();
        let hotspot = cursor.hotSpot();
        let encoded = native_image
            .TIFFRepresentation()
            .ok_or(MacOsCaptureError::CursorMetadataUnavailable)?;
        let mut fingerprint = Context::new(&SHA256);
        let encoded: &objc2_core_foundation::CFData = encoded.as_ref();
        fingerprint.update(&encoded.to_vec());
        fingerprint.update(&image_size.width.to_bits().to_le_bytes());
        fingerprint.update(&image_size.height.to_bits().to_le_bytes());
        fingerprint.update(&hotspot.x.to_bits().to_le_bytes());
        fingerprint.update(&hotspot.y.to_bits().to_le_bytes());
        let fingerprint: [u8; 32] = fingerprint
            .finish()
            .as_ref()
            .try_into()
            .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?;

        let changed_image = if self.last_fingerprint == Some(fingerprint) {
            None
        } else {
            let revision = self
                .revision
                .checked_add(1)
                .ok_or(MacOsCaptureError::CursorRevisionExhausted)?;
            let image = decode_cursor_image(
                encoded,
                image_size.width,
                image_size.height,
                hotspot.x,
                hotspot.y,
                revision,
            )?;
            self.last_fingerprint = Some(fingerprint);
            self.revision = revision;
            Some(image)
        };

        Ok(CursorSample {
            visible,
            desktop_x,
            desktop_y,
            primary_click,
            secondary_click,
            image_revision: self.revision,
            changed_image,
        })
    }
}

impl CursorMetadataSession {
    pub(super) fn new(
        target: ScreenTargetDescriptor,
        output: VideoFrameSpec,
        policy: CursorPolicy,
    ) -> Result<Self, MacOsCaptureError> {
        if policy.mode() != CursorCaptureMode::Metadata {
            return Err(MacOsCaptureError::UnsupportedCursorMode);
        }
        Ok(Self {
            target,
            output,
            policy,
            sampler: CursorSampler::new(),
            pending_image: None,
            published_revision: None,
        })
    }

    pub(super) fn active_events(
        &mut self,
        stream: ScreenStreamStamp,
        frame: MacOsCaptureFrame,
    ) -> Result<CursorEvents, MacOsCaptureError> {
        let sample = self.sampler.sample(self.policy.include_image_revision())?;
        if let Some(image) = sample.changed_image {
            self.pending_image = Some(image);
        }
        let cursor = normalize_screen_cursor(
            &self.target,
            self.output,
            self.policy,
            RawCursorObservation {
                visible: sample.visible,
                position: cursor_position(
                    &self.target,
                    self.output,
                    sample.desktop_x,
                    sample.desktop_y,
                )?,
                image_revision: (sample.image_revision > 0).then_some(sample.image_revision),
                primary_click: sample.primary_click,
                secondary_click: sample.secondary_click,
            },
        )
        .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?;

        let visible_revision = cursor
            .filter(|metadata| metadata.visible())
            .and_then(|metadata| metadata.image_revision());
        let mut events = Vec::with_capacity(2);
        if let Some(revision) =
            visible_revision.filter(|revision| self.published_revision != Some(*revision))
        {
            let image = self
                .pending_image
                .take()
                .filter(|image| image.revision == revision)
                .ok_or(MacOsCaptureError::CursorMetadataUnavailable)?;
            events.push(ScreenSourceEvent::CursorImage(screen_cursor_image(
                stream, image,
            )?));
            self.published_revision = Some(revision);
        }
        events.push(ScreenSourceEvent::Frame(normalized_frame(
            stream, frame, cursor,
        )?));
        Ok(events)
    }

    pub(super) fn stopped_frame(
        &self,
        stream: ScreenStreamStamp,
        frame: MacOsCaptureFrame,
    ) -> Result<ScreenFrame<Box<[u8]>>, MacOsCaptureError> {
        let cursor = normalize_screen_cursor(
            &self.target,
            self.output,
            self.policy,
            RawCursorObservation {
                visible: false,
                position: RawCursorPosition::TargetFramePhysical { x: 0, y: 0 },
                image_revision: None,
                primary_click: false,
                secondary_click: false,
            },
        )
        .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?;
        normalized_frame(stream, frame, cursor)
    }
}

fn cursor_position(
    target: &ScreenTargetDescriptor,
    output: VideoFrameSpec,
    desktop_x: i32,
    desktop_y: i32,
) -> Result<RawCursorPosition, MacOsCaptureError> {
    if target.kind() != ScreenTargetKind::Window {
        return Ok(RawCursorPosition::DesktopLogical {
            x: desktop_x,
            y: desktop_y,
        });
    }
    let bounds = target.logical_bounds();
    if !bounds.contains_point(desktop_x, desktop_y) {
        return Ok(RawCursorPosition::TargetFramePhysical {
            x: output.width,
            y: output.height,
        });
    }
    let local_x = u64::try_from(i64::from(desktop_x) - i64::from(bounds.x()))
        .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?;
    let local_y = u64::try_from(i64::from(desktop_y) - i64::from(bounds.y()))
        .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?;
    let frame_x = local_x
        .checked_mul(u64::from(output.width))
        .ok_or(MacOsCaptureError::CursorMetadataUnavailable)?
        / u64::from(bounds.width());
    let frame_y = local_y
        .checked_mul(u64::from(output.height))
        .ok_or(MacOsCaptureError::CursorMetadataUnavailable)?
        / u64::from(bounds.height());
    Ok(RawCursorPosition::TargetFramePhysical {
        x: u32::try_from(frame_x).map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?,
        y: u32::try_from(frame_y).map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?,
    })
}

fn screen_cursor_image(
    stream: ScreenStreamStamp,
    image: CursorImage,
) -> Result<ScreenCursorImage<Box<[u8]>>, MacOsCaptureError> {
    let retained_bytes =
        u64::try_from(image.rgba.len()).map_err(|_| MacOsCaptureError::CursorImageExceedsLimit)?;
    let descriptor = CursorImageDescriptor::new(
        image.revision,
        image.width,
        image.height,
        image.hotspot_x,
        image.hotspot_y,
        PixelFormat::Rgba8,
        retained_bytes,
    )
    .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?;
    ScreenCursorImage::new(stream, descriptor, image.rgba)
        .map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)
}

fn normalized_frame(
    stream: ScreenStreamStamp,
    frame: MacOsCaptureFrame,
    cursor: Option<frame_media::CursorFrameMetadata>,
) -> Result<ScreenFrame<Box<[u8]>>, MacOsCaptureError> {
    let sequence = frame.sequence();
    let timestamp = frame.timestamp();
    let spec = frame.spec();
    let payload = frame.into_pixels().into_boxed_slice();
    let retained_bytes =
        u64::try_from(payload.len()).map_err(|_| MacOsCaptureError::FrameAllocationExceedsLimit)?;
    ScreenFrame::new(
        ScreenFrameEnvelope {
            stream,
            sequence,
            timestamp,
            spec,
            retained_bytes,
            cursor,
        },
        payload,
    )
    .map_err(|_| MacOsCaptureError::InvalidSampleBuffer)
}

fn decode_cursor_image(
    encoded: &objc2_core_foundation::CFData,
    point_width: f64,
    point_height: f64,
    hotspot_x: f64,
    hotspot_y: f64,
    revision: u64,
) -> Result<CursorImage, MacOsCaptureError> {
    let data: &objc2_foundation::NSData = encoded.as_ref();
    let bitmap = NSBitmapImageRep::imageRepWithData(data)
        .ok_or(MacOsCaptureError::CursorMetadataUnavailable)?;
    let width = u16::try_from(bitmap.pixelsWide())
        .map_err(|_| MacOsCaptureError::CursorImageExceedsLimit)?;
    let height = u16::try_from(bitmap.pixelsHigh())
        .map_err(|_| MacOsCaptureError::CursorImageExceedsLimit)?;
    if width == 0
        || height == 0
        || width > MAX_CURSOR_DIMENSION
        || height > MAX_CURSOR_DIMENSION
        || !point_width.is_finite()
        || !point_height.is_finite()
        || point_width <= 0.0
        || point_height <= 0.0
    {
        return Err(MacOsCaptureError::CursorImageExceedsLimit);
    }
    let byte_count = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
        .ok_or(MacOsCaptureError::CursorImageExceedsLimit)?;
    let color_space = NSColorSpace::sRGBColorSpace();
    let mut rgba = Vec::with_capacity(byte_count);
    for y in 0..height {
        for x in 0..width {
            let color = bitmap
                .colorAtX_y(
                    isize::try_from(x).map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?,
                    isize::try_from(y).map_err(|_| MacOsCaptureError::CursorMetadataUnavailable)?,
                )
                .and_then(|color| color.colorUsingColorSpace(&color_space))
                .ok_or(MacOsCaptureError::CursorMetadataUnavailable)?;
            rgba.extend_from_slice(&[
                component(color.redComponent())?,
                component(color.greenComponent())?,
                component(color.blueComponent())?,
                component(color.alphaComponent())?,
            ]);
        }
    }
    if rgba.len() != byte_count {
        return Err(MacOsCaptureError::CursorMetadataUnavailable);
    }
    let hotspot_x = scaled_hotspot(hotspot_x, point_width, width)?;
    let hotspot_y = scaled_hotspot(hotspot_y, point_height, height)?;
    Ok(CursorImage {
        revision,
        width,
        height,
        hotspot_x,
        hotspot_y,
        rgba: rgba.into_boxed_slice(),
    })
}

fn checked_coordinate(value: f64) -> Result<i32, MacOsCaptureError> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(MacOsCaptureError::CursorMetadataUnavailable);
    }
    Ok(value.floor() as i32)
}

fn component(value: f64) -> Result<u8, MacOsCaptureError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(MacOsCaptureError::CursorMetadataUnavailable);
    }
    Ok((value * 255.0).round() as u8)
}

fn scaled_hotspot(
    coordinate: f64,
    point_extent: f64,
    pixel_extent: u16,
) -> Result<u16, MacOsCaptureError> {
    if !coordinate.is_finite() || coordinate < 0.0 || coordinate >= point_extent {
        return Err(MacOsCaptureError::CursorMetadataUnavailable);
    }
    let scaled = (coordinate * f64::from(pixel_extent) / point_extent).floor();
    if scaled < 0.0 || scaled >= f64::from(pixel_extent) {
        return Err(MacOsCaptureError::CursorMetadataUnavailable);
    }
    Ok(scaled as u16)
}

#[cfg(test)]
mod tests {
    use frame_media::{
        DisplayGeometryTransform, DpiScale, LogicalRect, PhysicalRect, Rotation,
        ScreenSourceInstanceId, ScreenTargetBinding, ScreenTargetDescriptor, ScreenTargetEpoch,
        ScreenTargetId,
    };

    use super::*;

    #[test]
    fn coordinate_and_component_validation_are_bounded() {
        assert_eq!(checked_coordinate(-1.25), Ok(-2));
        assert_eq!(component(0.5), Ok(128));
        assert_eq!(scaled_hotspot(4.0, 16.0, 32), Ok(8));
        assert!(checked_coordinate(f64::NAN).is_err());
        assert!(component(1.01).is_err());
        assert!(scaled_hotspot(16.0, 16.0, 32).is_err());
    }

    #[test]
    fn window_coordinates_are_clipped_and_scaled_before_export() {
        let source = ScreenSourceInstanceId::new([1; 16]).expect("source");
        let binding = ScreenTargetBinding::new(
            source,
            1,
            ScreenTargetEpoch::new(1).expect("epoch"),
            ScreenTargetId::new(ScreenTargetKind::Window, [2; 16]).expect("target"),
        )
        .expect("binding");
        let target = ScreenTargetDescriptor::window(
            binding,
            LogicalRect::new(-100, 50, 400, 200).expect("bounds"),
        )
        .expect("window");
        let output = VideoFrameSpec {
            width: 800,
            height: 400,
            pixel_format: PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            nominal_frame_duration_ns: 16_666_667,
            memory: frame_media::FrameMemory::Cpu,
        };
        assert_eq!(
            cursor_position(&target, output, 100, 150),
            Ok(RawCursorPosition::TargetFramePhysical { x: 400, y: 200 })
        );
        assert_eq!(
            cursor_position(&target, output, -101, 150),
            Ok(RawCursorPosition::TargetFramePhysical { x: 800, y: 400 })
        );

        let display_binding = ScreenTargetBinding::new(
            source,
            1,
            ScreenTargetEpoch::new(1).expect("epoch"),
            ScreenTargetId::new(ScreenTargetKind::Display, [3; 16]).expect("display"),
        )
        .expect("binding");
        let display = ScreenTargetDescriptor::display(
            display_binding,
            DisplayGeometryTransform::new(
                LogicalRect::new(0, 0, 800, 600).expect("logical"),
                PhysicalRect::new(0, 0, 1_600, 1_200).expect("physical"),
                DpiScale::new(2, 1).expect("scale"),
                Rotation::Degrees0,
            )
            .expect("transform"),
        )
        .expect("display");
        assert_eq!(
            cursor_position(&display, output, 40, 50),
            Ok(RawCursorPosition::DesktopLogical { x: 40, y: 50 })
        );
    }
}
