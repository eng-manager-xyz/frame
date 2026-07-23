//! Normalized Windows cursor metadata over the audited Win32 FFI boundary.

use frame_media::{
    CursorCaptureMode, CursorImageDescriptor, CursorPolicy, PixelFormat, RawCursorObservation,
    RawCursorPosition, ScreenCursorImage, ScreenFrame, ScreenFrameEnvelope, ScreenSourceEvent,
    ScreenStreamStamp, ScreenTargetDescriptor, ScreenTargetKind, ScreenTargetSnapshot,
    VideoFrameSpec, normalize_screen_cursor,
};
use frame_windows_capture_ffi::{WindowsCursorImage, WindowsCursorSampler};

use crate::{WindowsCaptureError, WindowsCaptureFrame};

type CursorEvents = Vec<ScreenSourceEvent<Box<[u8]>, Box<[u8]>>>;

pub(super) struct CursorMetadataSession {
    target: ScreenTargetDescriptor,
    output: VideoFrameSpec,
    policy: CursorPolicy,
    mapping: CursorMapping,
    sampler: WindowsCursorSampler,
    pending_image: Option<WindowsCursorImage>,
    published_revision: Option<u64>,
}

#[derive(Clone, Copy)]
struct CursorMapping {
    desktop_x: i32,
    desktop_y: i32,
    width: u32,
    height: u32,
}

impl CursorMetadataSession {
    pub(super) fn new(
        target: ScreenTargetDescriptor,
        catalog: &ScreenTargetSnapshot,
        output: VideoFrameSpec,
        policy: CursorPolicy,
    ) -> Result<Self, WindowsCaptureError> {
        if policy.mode() != CursorCaptureMode::Metadata {
            return Err(WindowsCaptureError::UnsupportedCursorMode);
        }
        let mapping = CursorMapping::for_target(&target, catalog)?;
        Ok(Self {
            target,
            output,
            policy,
            mapping,
            sampler: WindowsCursorSampler::new(),
            pending_image: None,
            published_revision: None,
        })
    }

    pub(super) fn active_events(
        &mut self,
        stream: ScreenStreamStamp,
        frame: WindowsCaptureFrame,
    ) -> Result<CursorEvents, WindowsCaptureError> {
        let mut sample = self
            .sampler
            .sample(self.policy.include_image_revision())
            .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?;
        if let Some(image) = sample.take_changed_image() {
            self.pending_image = Some(image);
        }
        let (desktop_x, desktop_y) = sample.desktop_position();
        let cursor = normalize_screen_cursor(
            &self.target,
            self.output,
            self.policy,
            RawCursorObservation {
                visible: sample.visible(),
                position: self
                    .mapping
                    .frame_position(self.output, desktop_x, desktop_y)?,
                image_revision: sample.image_revision(),
                primary_click: sample.primary_click(),
                secondary_click: sample.secondary_click(),
            },
        )
        .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?;

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
                .filter(|image| image.revision() == revision)
                .ok_or(WindowsCaptureError::CursorMetadataUnavailable)?;
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
        frame: WindowsCaptureFrame,
    ) -> Result<ScreenFrame<Box<[u8]>>, WindowsCaptureError> {
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
        .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?;
        normalized_frame(stream, frame, cursor)
    }
}

impl CursorMapping {
    fn for_target(
        target: &ScreenTargetDescriptor,
        catalog: &ScreenTargetSnapshot,
    ) -> Result<Self, WindowsCaptureError> {
        let bounds = match target.kind() {
            ScreenTargetKind::Display => target
                .display_transform()
                .ok_or(WindowsCaptureError::CursorMetadataUnavailable)?
                .physical_bounds(),
            ScreenTargetKind::Window => {
                let bounds = target.logical_bounds();
                frame_media::PhysicalRect::new(
                    bounds.x(),
                    bounds.y(),
                    bounds.width(),
                    bounds.height(),
                )
                .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?
            }
            ScreenTargetKind::Region => {
                let display = target
                    .containing_display_binding()
                    .and_then(|binding| catalog.find_binding(binding))
                    .and_then(ScreenTargetDescriptor::display_transform)
                    .ok_or(WindowsCaptureError::CursorMetadataUnavailable)?;
                display
                    .logical_rect_to_physical(target.logical_bounds())
                    .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?
            }
        };
        Ok(Self {
            desktop_x: bounds.x(),
            desktop_y: bounds.y(),
            width: bounds.width(),
            height: bounds.height(),
        })
    }

    fn frame_position(
        self,
        output: VideoFrameSpec,
        desktop_x: i32,
        desktop_y: i32,
    ) -> Result<RawCursorPosition, WindowsCaptureError> {
        let local_x = i64::from(desktop_x) - i64::from(self.desktop_x);
        let local_y = i64::from(desktop_y) - i64::from(self.desktop_y);
        if local_x < 0
            || local_y < 0
            || local_x >= i64::from(self.width)
            || local_y >= i64::from(self.height)
        {
            return Ok(RawCursorPosition::TargetFramePhysical {
                x: output.width,
                y: output.height,
            });
        }
        let frame_x = u64::try_from(local_x)
            .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?
            .checked_mul(u64::from(output.width))
            .ok_or(WindowsCaptureError::CursorMetadataUnavailable)?
            / u64::from(self.width);
        let frame_y = u64::try_from(local_y)
            .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?
            .checked_mul(u64::from(output.height))
            .ok_or(WindowsCaptureError::CursorMetadataUnavailable)?
            / u64::from(self.height);
        Ok(RawCursorPosition::TargetFramePhysical {
            x: u32::try_from(frame_x)
                .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?,
            y: u32::try_from(frame_y)
                .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?,
        })
    }
}

fn screen_cursor_image(
    stream: ScreenStreamStamp,
    image: WindowsCursorImage,
) -> Result<ScreenCursorImage<Box<[u8]>>, WindowsCaptureError> {
    let revision = image.revision();
    let (width, height) = image.dimensions();
    let (hotspot_x, hotspot_y) = image.hotspot();
    let pixels = image.into_bgra();
    let retained_bytes =
        u64::try_from(pixels.len()).map_err(|_| WindowsCaptureError::CursorImageExceedsLimit)?;
    let descriptor = CursorImageDescriptor::new(
        revision,
        width,
        height,
        hotspot_x,
        hotspot_y,
        PixelFormat::Bgra8,
        retained_bytes,
    )
    .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)?;
    ScreenCursorImage::new(stream, descriptor, pixels)
        .map_err(|_| WindowsCaptureError::CursorMetadataUnavailable)
}

fn normalized_frame(
    stream: ScreenStreamStamp,
    frame: WindowsCaptureFrame,
    cursor: Option<frame_media::CursorFrameMetadata>,
) -> Result<ScreenFrame<Box<[u8]>>, WindowsCaptureError> {
    let sequence = frame.sequence();
    let timestamp = frame.timestamp();
    let spec = frame.spec();
    let payload = frame.into_pixels().into_boxed_slice();
    let retained_bytes = u64::try_from(payload.len())
        .map_err(|_| WindowsCaptureError::FrameAllocationExceedsLimit)?;
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
    .map_err(|_| WindowsCaptureError::InvalidNativeFrame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_mapping_scales_and_clips_without_signed_wraparound() {
        let mapping = CursorMapping {
            desktop_x: -1_920,
            desktop_y: -100,
            width: 1_920,
            height: 1_080,
        };
        let output = VideoFrameSpec {
            width: 960,
            height: 540,
            pixel_format: PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            nominal_frame_duration_ns: 16_666_667,
            memory: frame_media::FrameMemory::Cpu,
        };
        assert_eq!(
            mapping.frame_position(output, -960, 440),
            Ok(RawCursorPosition::TargetFramePhysical { x: 480, y: 270 })
        );
        assert_eq!(
            mapping.frame_position(output, -1_921, 0),
            Ok(RawCursorPosition::TargetFramePhysical { x: 960, y: 540 })
        );
    }
}
