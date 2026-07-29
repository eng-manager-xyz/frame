//! Durable, provider-neutral cursor metadata for Studio recordings.
//!
//! Platform capture adapters normalize cursor observations to the negotiated
//! screen frame before they reach this module. The sidecar is append-friendly:
//! one fixed header is followed by length-framed observations, and cursor
//! pixels are carried only when an image revision changes. The complete file
//! is still authenticated by the enclosing [`crate::StudioAsset`] checksum.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::{FrameTimestamp, MAX_CURSOR_IMAGE_DIMENSION, PixelFormat};

pub const STUDIO_CURSOR_TIMELINE_VERSION: u16 = 1;
pub const MAX_STUDIO_CURSOR_OBSERVATIONS: usize = 2_000_000;
pub const MAX_STUDIO_CURSOR_TIMELINE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_STUDIO_CURSOR_IMAGE_BYTES: usize = 8 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"FRCU";
const HEADER_BYTES: usize = 16;
const MINIMUM_OBSERVATION_BYTES: usize = 41;
const MAXIMUM_RECORD_BYTES: usize = MAX_STUDIO_CURSOR_IMAGE_BYTES + 64;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum StudioCursorError {
    #[error("Studio cursor timeline is malformed")]
    Malformed,
    #[error("Studio cursor timeline exceeds its resource bound")]
    ResourceLimit,
    #[error("Studio cursor observations are not monotonic")]
    NonMonotonic,
    #[error("Studio cursor image is invalid")]
    InvalidImage,
    #[error("Studio cursor observation is invalid")]
    InvalidObservation,
}

/// A complete cursor bitmap associated with a monotonically increasing native
/// image revision. Pixels are tightly packed in row-major order.
#[derive(Clone, PartialEq, Eq)]
pub struct StudioCursorImage {
    revision: u64,
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    pixel_format: PixelFormat,
    pixels: Vec<u8>,
}

impl StudioCursorImage {
    pub fn new(
        revision: u64,
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        pixel_format: PixelFormat,
        pixels: Vec<u8>,
    ) -> Result<Self, StudioCursorError> {
        let expected = usize::from(width)
            .checked_mul(usize::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(StudioCursorError::InvalidImage)?;
        if revision == 0
            || width == 0
            || height == 0
            || width > MAX_CURSOR_IMAGE_DIMENSION
            || height > MAX_CURSOR_IMAGE_DIMENSION
            || hotspot_x >= width
            || hotspot_y >= height
            || !matches!(pixel_format, PixelFormat::Bgra8 | PixelFormat::Rgba8)
            || expected == 0
            || expected > MAX_STUDIO_CURSOR_IMAGE_BYTES
            || pixels.len() != expected
        {
            return Err(StudioCursorError::InvalidImage);
        }
        Ok(Self {
            revision,
            width,
            height,
            hotspot_x,
            hotspot_y,
            pixel_format,
            pixels,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn hotspot(&self) -> (u16, u16) {
        (self.hotspot_x, self.hotspot_y)
    }

    #[must_use]
    pub const fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

impl std::fmt::Debug for StudioCursorImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StudioCursorImage")
            .field("revision", &self.revision)
            .field("dimensions", &self.dimensions())
            .field("hotspot", &self.hotspot())
            .field("pixel_format", &self.pixel_format)
            .field("pixel_bytes", &self.pixels.len())
            .finish()
    }
}

/// One normalized cursor observation. `image_update` is present only when the
/// referenced revision first appears in the stream.
#[derive(Clone, PartialEq, Eq)]
pub struct StudioCursorObservation {
    sequence: u64,
    timestamp: FrameTimestamp,
    visible: bool,
    frame_x: u32,
    frame_y: u32,
    image_revision: Option<u64>,
    primary_click: bool,
    secondary_click: bool,
    image_update: Option<StudioCursorImage>,
}

impl StudioCursorObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sequence: u64,
        timestamp: FrameTimestamp,
        visible: bool,
        frame_x: u32,
        frame_y: u32,
        image_revision: Option<u64>,
        primary_click: bool,
        secondary_click: bool,
        image_update: Option<StudioCursorImage>,
    ) -> Result<Self, StudioCursorError> {
        if sequence == 0
            || timestamp.duration_ns == 0
            || timestamp
                .pts_ns
                .checked_add(timestamp.duration_ns)
                .is_none()
            || image_revision == Some(0)
            || image_update
                .as_ref()
                .is_some_and(|image| Some(image.revision()) != image_revision)
            || (!visible
                && (frame_x != 0
                    || frame_y != 0
                    || image_revision.is_some()
                    || primary_click
                    || secondary_click
                    || image_update.is_some()))
        {
            return Err(StudioCursorError::InvalidObservation);
        }
        Ok(Self {
            sequence,
            timestamp,
            visible,
            frame_x,
            frame_y,
            image_revision,
            primary_click,
            secondary_click,
            image_update,
        })
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn timestamp(&self) -> FrameTimestamp {
        self.timestamp
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub const fn frame_position(&self) -> Option<(u32, u32)> {
        if self.visible {
            Some((self.frame_x, self.frame_y))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn image_revision(&self) -> Option<u64> {
        self.image_revision
    }

    #[must_use]
    pub const fn primary_click(&self) -> bool {
        self.primary_click
    }

    #[must_use]
    pub const fn secondary_click(&self) -> bool {
        self.secondary_click
    }

    #[must_use]
    pub const fn image_update(&self) -> Option<&StudioCursorImage> {
        self.image_update.as_ref()
    }
}

impl std::fmt::Debug for StudioCursorObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StudioCursorObservation")
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field("visible", &self.visible)
            .field("image_revision", &self.image_revision)
            .field("primary_click", &self.primary_click)
            .field("secondary_click", &self.secondary_click)
            .field("has_image_update", &self.image_update.is_some())
            .finish()
    }
}

/// A validated cursor sidecar indexed for bounded point lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioCursorTimeline {
    frame_width: u32,
    frame_height: u32,
    observations: Vec<StudioCursorObservation>,
    images: BTreeMap<u64, StudioCursorImage>,
}

impl StudioCursorTimeline {
    pub fn new(
        frame_width: u32,
        frame_height: u32,
        observations: Vec<StudioCursorObservation>,
    ) -> Result<Self, StudioCursorError> {
        let pixels = u64::from(frame_width)
            .checked_mul(u64::from(frame_height))
            .ok_or(StudioCursorError::ResourceLimit)?;
        if frame_width == 0
            || frame_height == 0
            || pixels > 132_710_400
            || observations.is_empty()
            || observations.len() > MAX_STUDIO_CURSOR_OBSERVATIONS
        {
            return Err(StudioCursorError::InvalidObservation);
        }
        let mut previous_sequence = None;
        let mut previous_end = None;
        let mut last_image_revision = None;
        let mut images = BTreeMap::new();
        for observation in &observations {
            if previous_sequence.is_some_and(|value| observation.sequence <= value)
                || previous_end.is_some_and(|value| observation.timestamp.pts_ns < value)
            {
                return Err(StudioCursorError::NonMonotonic);
            }
            if observation.visible
                && (observation.frame_x >= frame_width || observation.frame_y >= frame_height)
            {
                return Err(StudioCursorError::InvalidObservation);
            }
            if let Some(image) = observation.image_update.as_ref() {
                if last_image_revision.is_some_and(|revision| image.revision() <= revision)
                    || images.insert(image.revision(), image.clone()).is_some()
                {
                    return Err(StudioCursorError::InvalidImage);
                }
                last_image_revision = Some(image.revision());
            }
            if observation
                .image_revision
                .is_some_and(|revision| !images.contains_key(&revision))
            {
                return Err(StudioCursorError::InvalidObservation);
            }
            previous_sequence = Some(observation.sequence);
            previous_end = observation
                .timestamp
                .pts_ns
                .checked_add(observation.timestamp.duration_ns);
        }
        Ok(Self {
            frame_width,
            frame_height,
            observations,
            images,
        })
    }

    #[must_use]
    pub const fn frame_dimensions(&self) -> (u32, u32) {
        (self.frame_width, self.frame_height)
    }

    #[must_use]
    pub fn observations(&self) -> &[StudioCursorObservation] {
        &self.observations
    }

    #[must_use]
    pub fn image(&self, revision: u64) -> Option<&StudioCursorImage> {
        self.images.get(&revision)
    }

    /// Returns the most recent observation whose timestamp is not after the
    /// requested source time. This is logarithmic and remains bounded for the
    /// maximum supported day-scale sidecar.
    #[must_use]
    pub fn observation_at_ns(&self, source_time_ns: u64) -> Option<&StudioCursorObservation> {
        let index = self
            .observations
            .partition_point(|observation| observation.timestamp.pts_ns <= source_time_ns);
        index
            .checked_sub(1)
            .and_then(|index| self.observations.get(index))
    }

    pub fn encode(&self) -> Result<Vec<u8>, StudioCursorError> {
        let mut output = studio_cursor_timeline_header(self.frame_width, self.frame_height)?;
        for observation in &self.observations {
            output.extend_from_slice(&encode_studio_cursor_record(observation)?);
            if output.len() > MAX_STUDIO_CURSOR_TIMELINE_BYTES {
                return Err(StudioCursorError::ResourceLimit);
            }
        }
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StudioCursorError> {
        if bytes.len() < HEADER_BYTES
            || bytes.len() > MAX_STUDIO_CURSOR_TIMELINE_BYTES
            || bytes[..4] != MAGIC
            || u16::from_be_bytes([bytes[4], bytes[5]]) != STUDIO_CURSOR_TIMELINE_VERSION
            || bytes[6..8] != [0, 0]
        {
            return Err(StudioCursorError::Malformed);
        }
        let frame_width = u32::from_be_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| StudioCursorError::Malformed)?,
        );
        let frame_height = u32::from_be_bytes(
            bytes[12..16]
                .try_into()
                .map_err(|_| StudioCursorError::Malformed)?,
        );
        let mut cursor = HEADER_BYTES;
        let mut observations = Vec::new();
        while cursor < bytes.len() {
            let length_end = cursor
                .checked_add(4)
                .filter(|end| *end <= bytes.len())
                .ok_or(StudioCursorError::Malformed)?;
            let record_len = usize::try_from(u32::from_be_bytes(
                bytes[cursor..length_end]
                    .try_into()
                    .map_err(|_| StudioCursorError::Malformed)?,
            ))
            .map_err(|_| StudioCursorError::ResourceLimit)?;
            if !(MINIMUM_OBSERVATION_BYTES..=MAXIMUM_RECORD_BYTES).contains(&record_len) {
                return Err(StudioCursorError::Malformed);
            }
            let record_end = length_end
                .checked_add(record_len)
                .filter(|end| *end <= bytes.len())
                .ok_or(StudioCursorError::Malformed)?;
            observations.push(decode_studio_cursor_record(&bytes[cursor..record_end])?);
            if observations.len() > MAX_STUDIO_CURSOR_OBSERVATIONS {
                return Err(StudioCursorError::ResourceLimit);
            }
            cursor = record_end;
        }
        Self::new(frame_width, frame_height, observations)
    }
}

pub fn studio_cursor_timeline_header(
    frame_width: u32,
    frame_height: u32,
) -> Result<Vec<u8>, StudioCursorError> {
    let pixels = u64::from(frame_width)
        .checked_mul(u64::from(frame_height))
        .ok_or(StudioCursorError::ResourceLimit)?;
    if frame_width == 0 || frame_height == 0 || pixels > 132_710_400 {
        return Err(StudioCursorError::InvalidObservation);
    }
    let mut output = Vec::with_capacity(HEADER_BYTES);
    output.extend_from_slice(&MAGIC);
    output.extend_from_slice(&STUDIO_CURSOR_TIMELINE_VERSION.to_be_bytes());
    output.extend_from_slice(&0_u16.to_be_bytes());
    output.extend_from_slice(&frame_width.to_be_bytes());
    output.extend_from_slice(&frame_height.to_be_bytes());
    Ok(output)
}

pub fn encode_studio_cursor_record(
    observation: &StudioCursorObservation,
) -> Result<Vec<u8>, StudioCursorError> {
    let record = encode_observation(observation)?;
    let length = u32::try_from(record.len()).map_err(|_| StudioCursorError::ResourceLimit)?;
    let mut output = Vec::with_capacity(record.len() + 4);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&record);
    Ok(output)
}

pub fn decode_studio_cursor_record(
    bytes: &[u8],
) -> Result<StudioCursorObservation, StudioCursorError> {
    if bytes.len() < 4 {
        return Err(StudioCursorError::Malformed);
    }
    let length = usize::try_from(u32::from_be_bytes(
        bytes[..4]
            .try_into()
            .map_err(|_| StudioCursorError::Malformed)?,
    ))
    .map_err(|_| StudioCursorError::ResourceLimit)?;
    if !(MINIMUM_OBSERVATION_BYTES..=MAXIMUM_RECORD_BYTES).contains(&length)
        || length.checked_add(4) != Some(bytes.len())
    {
        return Err(StudioCursorError::Malformed);
    }
    decode_observation(&bytes[4..])
}

fn encode_observation(observation: &StudioCursorObservation) -> Result<Vec<u8>, StudioCursorError> {
    let mut flags = 0_u8;
    flags |= u8::from(observation.visible);
    flags |= u8::from(observation.primary_click) << 1;
    flags |= u8::from(observation.secondary_click) << 2;
    flags |= u8::from(observation.image_revision.is_some()) << 3;
    flags |= u8::from(observation.image_update.is_some()) << 4;
    flags |= u8::from(observation.timestamp.discontinuity) << 5;
    let mut output = Vec::with_capacity(MINIMUM_OBSERVATION_BYTES);
    output.extend_from_slice(&observation.sequence.to_be_bytes());
    output.extend_from_slice(&observation.timestamp.pts_ns.to_be_bytes());
    output.extend_from_slice(&observation.timestamp.duration_ns.to_be_bytes());
    output.push(flags);
    output.extend_from_slice(&observation.frame_x.to_be_bytes());
    output.extend_from_slice(&observation.frame_y.to_be_bytes());
    output.extend_from_slice(&observation.image_revision.unwrap_or(0).to_be_bytes());
    if let Some(image) = &observation.image_update {
        output.extend_from_slice(&image.width.to_be_bytes());
        output.extend_from_slice(&image.height.to_be_bytes());
        output.extend_from_slice(&image.hotspot_x.to_be_bytes());
        output.extend_from_slice(&image.hotspot_y.to_be_bytes());
        output.push(match image.pixel_format {
            PixelFormat::Bgra8 => 1,
            PixelFormat::Rgba8 => 2,
            PixelFormat::Nv12 | PixelFormat::I420 => {
                return Err(StudioCursorError::InvalidImage);
            }
        });
        let length =
            u32::try_from(image.pixels.len()).map_err(|_| StudioCursorError::ResourceLimit)?;
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(&image.pixels);
    }
    Ok(output)
}

fn decode_observation(bytes: &[u8]) -> Result<StudioCursorObservation, StudioCursorError> {
    let mut reader = CursorReader::new(bytes);
    let sequence = reader.u64()?;
    let pts_ns = reader.u64()?;
    let duration_ns = reader.u64()?;
    let flags = reader.u8()?;
    if flags & !0b11_1111 != 0 {
        return Err(StudioCursorError::Malformed);
    }
    let timestamp = FrameTimestamp {
        pts_ns,
        duration_ns,
        discontinuity: flags & (1 << 5) != 0,
    };
    let visible = flags & 1 != 0;
    let primary_click = flags & (1 << 1) != 0;
    let secondary_click = flags & (1 << 2) != 0;
    let has_revision = flags & (1 << 3) != 0;
    let has_image = flags & (1 << 4) != 0;
    let frame_x = reader.u32()?;
    let frame_y = reader.u32()?;
    let revision = reader.u64()?;
    let image_revision = has_revision.then_some(revision);
    if has_revision == (revision == 0) || (has_image && !has_revision) {
        return Err(StudioCursorError::Malformed);
    }
    let image_update = if has_image {
        let width = reader.u16()?;
        let height = reader.u16()?;
        let hotspot_x = reader.u16()?;
        let hotspot_y = reader.u16()?;
        let pixel_format = match reader.u8()? {
            1 => PixelFormat::Bgra8,
            2 => PixelFormat::Rgba8,
            _ => return Err(StudioCursorError::Malformed),
        };
        let length =
            usize::try_from(reader.u32()?).map_err(|_| StudioCursorError::ResourceLimit)?;
        let pixels = reader.bytes(length)?.to_vec();
        Some(StudioCursorImage::new(
            revision,
            width,
            height,
            hotspot_x,
            hotspot_y,
            pixel_format,
            pixels,
        )?)
    } else {
        None
    };
    reader.finish()?;
    StudioCursorObservation::new(
        sequence,
        timestamp,
        visible,
        frame_x,
        frame_y,
        image_revision,
        primary_click,
        secondary_click,
        image_update,
    )
}

struct CursorReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> CursorReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn bytes(&mut self, length: usize) -> Result<&'a [u8], StudioCursorError> {
        let end = self
            .cursor
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(StudioCursorError::Malformed)?;
        let value = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, StudioCursorError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, StudioCursorError> {
        Ok(u16::from_be_bytes(
            self.bytes(2)?
                .try_into()
                .map_err(|_| StudioCursorError::Malformed)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, StudioCursorError> {
        Ok(u32::from_be_bytes(
            self.bytes(4)?
                .try_into()
                .map_err(|_| StudioCursorError::Malformed)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, StudioCursorError> {
        Ok(u64::from_be_bytes(
            self.bytes(8)?
                .try_into()
                .map_err(|_| StudioCursorError::Malformed)?,
        ))
    }

    fn finish(self) -> Result<(), StudioCursorError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(StudioCursorError::Malformed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(revision: u64) -> StudioCursorImage {
        StudioCursorImage::new(
            revision,
            2,
            2,
            0,
            0,
            PixelFormat::Rgba8,
            vec![
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
            ],
        )
        .expect("cursor image")
    }

    fn observation(
        sequence: u64,
        pts_ns: u64,
        revision: u64,
        image_update: Option<StudioCursorImage>,
    ) -> StudioCursorObservation {
        StudioCursorObservation::new(
            sequence,
            FrameTimestamp {
                pts_ns,
                duration_ns: 10,
                discontinuity: false,
            },
            true,
            u32::try_from(sequence).expect("sequence fits"),
            3,
            Some(revision),
            sequence == 2,
            false,
            image_update,
        )
        .expect("observation")
    }

    #[test]
    fn timeline_round_trips_and_uses_latest_not_future_observation() {
        let timeline = StudioCursorTimeline::new(
            1920,
            1080,
            vec![
                observation(1, 10, 7, Some(image(7))),
                observation(2, 20, 7, None),
            ],
        )
        .expect("timeline");
        let encoded = timeline.encode().expect("encode");
        let decoded = StudioCursorTimeline::decode(&encoded).expect("decode");
        assert_eq!(decoded, timeline);
        assert!(decoded.observation_at_ns(9).is_none());
        assert_eq!(
            decoded.observation_at_ns(19).map(|value| value.sequence()),
            Some(1)
        );
        assert_eq!(
            decoded.observation_at_ns(20).map(|value| value.sequence()),
            Some(2)
        );
        assert_eq!(decoded.image(7), Some(&image(7)));
    }

    #[test]
    fn decoder_rejects_truncation_and_unknown_revisions() {
        let timeline =
            StudioCursorTimeline::new(100, 100, vec![observation(1, 10, 7, Some(image(7)))])
                .expect("timeline");
        let mut encoded = timeline.encode().expect("encode");
        encoded.pop();
        assert_eq!(
            StudioCursorTimeline::decode(&encoded),
            Err(StudioCursorError::Malformed)
        );
        assert_eq!(
            StudioCursorTimeline::new(100, 100, vec![observation(1, 10, 7, None)]),
            Err(StudioCursorError::InvalidObservation)
        );
    }

    #[test]
    fn constructor_rejects_out_of_order_and_hidden_metadata() {
        let first = observation(2, 20, 7, Some(image(7)));
        let second = observation(1, 10, 7, None);
        assert_eq!(
            StudioCursorTimeline::new(100, 100, vec![first, second]),
            Err(StudioCursorError::NonMonotonic)
        );
        assert_eq!(
            StudioCursorObservation::new(
                1,
                FrameTimestamp {
                    pts_ns: 0,
                    duration_ns: 1,
                    discontinuity: false,
                },
                false,
                1,
                0,
                None,
                false,
                false,
                None,
            ),
            Err(StudioCursorError::InvalidObservation)
        );
    }
}
