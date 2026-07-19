use frame_media::{ScreenTargetBinding, VideoFrameSpec};

use crate::{
    MacOsCaptureError, MacOsCaptureFrame, NormalizedTimestamp, RawMediaTime, TimestampNormalizer,
};

pub(super) struct FrameAssembly {
    pub(super) frame: MacOsCaptureFrame,
    pub(super) used_nominal_duration: bool,
}

pub(super) struct FrameAssembler {
    target: ScreenTargetBinding,
    spec: VideoFrameSpec,
    next_sequence: u64,
    timestamps: TimestampNormalizer,
    pending_discontinuity: bool,
    last_complete_pixels: Option<Vec<u8>>,
}

impl FrameAssembler {
    pub(super) const fn new(target: ScreenTargetBinding, spec: VideoFrameSpec) -> Self {
        Self {
            target,
            spec,
            next_sequence: 1,
            timestamps: TimestampNormalizer::new(),
            pending_discontinuity: false,
            last_complete_pixels: None,
        }
    }

    pub(super) const fn spec(&self) -> VideoFrameSpec {
        self.spec
    }

    pub(super) fn mark_non_content_discontinuity(&mut self) {
        self.pending_discontinuity = true;
    }

    pub(super) fn accept_complete(
        &mut self,
        pixels: Vec<u8>,
        pts: RawMediaTime,
        duration: RawMediaTime,
    ) -> Result<FrameAssembly, MacOsCaptureError> {
        let normalized = self.timestamps.normalize(
            pts,
            duration,
            self.spec.nominal_frame_duration_ns,
            self.pending_discontinuity,
        )?;
        let cached_pixels = pixels.clone();
        let assembly = self.assemble(pixels, normalized)?;
        self.last_complete_pixels = Some(cached_pixels);
        Ok(assembly)
    }

    pub(super) fn accept_idle(
        &mut self,
        pts: RawMediaTime,
    ) -> Result<Option<FrameAssembly>, MacOsCaptureError> {
        let Some(pixels) = self.last_complete_pixels.clone() else {
            return Ok(None);
        };
        let normalized = self.timestamps.normalize_nominal(
            pts,
            self.spec.nominal_frame_duration_ns,
            self.pending_discontinuity,
        )?;
        self.assemble(pixels, normalized).map(Some)
    }

    fn assemble(
        &mut self,
        pixels: Vec<u8>,
        normalized: NormalizedTimestamp,
    ) -> Result<FrameAssembly, MacOsCaptureError> {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(MacOsCaptureError::SequenceExhausted)?;
        self.pending_discontinuity = false;
        Ok(FrameAssembly {
            frame: MacOsCaptureFrame {
                target: self.target,
                sequence,
                source_pts_ns: normalized.source_pts_ns,
                timestamp: normalized.timestamp,
                spec: self.spec,
                pixels,
            },
            used_nominal_duration: normalized.used_nominal_duration,
        })
    }
}
