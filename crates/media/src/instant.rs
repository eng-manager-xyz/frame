//! Provider-neutral contracts for Instant recording.
//!
//! This module deliberately contains no filesystem, database, Cloudflare, or
//! GStreamer object. Native and hosted adapters implement the narrow ports
//! below. The core owns immutable media identity, bounded ownership, journal
//! fencing, retry scheduling, and publication reconciliation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    time::Duration,
};

use thiserror::Error;

pub const INSTANT_PROTOCOL_VERSION: u16 = 2;
pub const INSTANT_MANIFEST_VERSION: u16 = 1;
pub const INSTANT_JOURNAL_VERSION: u16 = 1;
pub const INSTANT_UPLOAD_VERSION: u16 = 1;
pub const INSTANT_FINALIZE_VERSION: u16 = 1;
pub const INSTANT_PROGRESS_VERSION: u16 = 1;
pub const MAX_INSTANT_SEGMENTS: u32 = 100_000;
pub const MAX_INSTANT_SEGMENT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_INSTANT_TRACKS: usize = 8;
pub const MAX_INSTANT_PAYLOAD_CHUNK_BYTES: usize = 1024 * 1024;
pub const MAX_INSTANT_OPERATION_RECEIPTS: usize = 600_000;
pub const MAX_INSTANT_JOURNAL_BYTES: usize = 256 * 1024 * 1024;

macro_rules! opaque_id {
    ($name:ident, $invalid:ident, $label:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            /// Constructs an identity from 128 bits produced by a CSPRNG at the
            /// caller boundary. All-zero input is reserved and rejected.
            pub fn from_csprng(bytes: [u8; 16]) -> Result<Self, InstantError> {
                if bytes.iter().all(|byte| *byte == 0) {
                    return Err(InstantError::$invalid);
                }
                Ok(Self(bytes))
            }

            #[allow(dead_code)]
            pub(crate) fn canonical_bytes(self) -> [u8; 16] {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($label, "(<redacted>)"))
            }
        }
    };
}

opaque_id!(InstantSessionId, InvalidSessionId, "InstantSessionId");
opaque_id!(InstantOperationId, InvalidOperationId, "InstantOperationId");
opaque_id!(InstantWorkerId, InvalidWorkerId, "InstantWorkerId");
opaque_id!(InstantUploadId, InvalidUploadId, "InstantUploadId");
opaque_id!(
    InstantPublicationId,
    InvalidPublicationId,
    "InstantPublicationId"
);
opaque_id!(InstantObjectId, InvalidObjectId, "InstantObjectId");
opaque_id!(InstantJobId, InvalidJobId, "InstantJobId");

/// A runtime-only reference to key material. It is intentionally non-cloneable,
/// has no serialization API, and never exposes key bytes.
pub struct RuntimeSpoolKeyHandle([u8; 16]);

impl RuntimeSpoolKeyHandle {
    pub fn from_runtime(bytes: [u8; 16]) -> Result<Self, InstantError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(InstantError::InvalidSpoolKeyHandle);
        }
        Ok(Self(bytes))
    }

    fn matches(&self, marker: Sha256Digest) -> bool {
        strong_sha256(&self.0) == marker
    }

    pub(crate) const fn canonical_bytes(&self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for RuntimeSpoolKeyHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeSpoolKeyHandle(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, InstantError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(InstantError::InvalidChecksum);
        }
        Ok(Self(bytes))
    }

    pub fn from_hex(value: &str) -> Result<Self, InstantError> {
        if value.len() != 64 || !value.is_ascii() {
            return Err(InstantError::InvalidChecksum);
        }
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
                .map_err(|_| InstantError::InvalidChecksum)?;
        }
        Self::from_bytes(bytes)
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(64);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }

    pub(crate) fn canonical_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Sha256Digest(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstantTrackRole {
    ScreenVideo,
    CameraVideo,
    MixedAudio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstantCodec {
    H264Avc,
    AacLowComplexity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantContainer {
    FragmentedMp4Cmaf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantTrackMetadata {
    track_number: u16,
    role: InstantTrackRole,
    codec: InstantCodec,
    timescale: u32,
    sample_count: u32,
    first_presentation_ns: u64,
    duration_ns: u64,
}

impl InstantTrackMetadata {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track_number: u16,
        role: InstantTrackRole,
        codec: InstantCodec,
        timescale: u32,
        sample_count: u32,
        first_presentation_ns: u64,
        duration_ns: u64,
    ) -> Result<Self, InstantError> {
        if track_number == 0 || timescale == 0 || sample_count == 0 || duration_ns == 0 {
            return Err(InstantError::InvalidTrackMetadata);
        }
        match (role, codec) {
            (
                InstantTrackRole::ScreenVideo | InstantTrackRole::CameraVideo,
                InstantCodec::H264Avc,
            )
            | (InstantTrackRole::MixedAudio, InstantCodec::AacLowComplexity) => {}
            _ => return Err(InstantError::TrackCodecMismatch),
        }
        Ok(Self {
            track_number,
            role,
            codec,
            timescale,
            sample_count,
            first_presentation_ns,
            duration_ns,
        })
    }

    #[must_use]
    pub const fn role(&self) -> InstantTrackRole {
        self.role
    }

    pub(crate) const fn track_number(&self) -> u16 {
        self.track_number
    }

    pub(crate) const fn codec(&self) -> InstantCodec {
        self.codec
    }

    pub(crate) const fn timescale(&self) -> u32 {
        self.timescale
    }

    pub(crate) const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub(crate) const fn first_presentation_ns(&self) -> u64 {
        self.first_presentation_ns
    }

    pub(crate) const fn duration_ns(&self) -> u64 {
        self.duration_ns
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct InstantSegmentDescriptor {
    protocol_version: u16,
    session_id: InstantSessionId,
    index: u32,
    start_ns: u64,
    duration_ns: u64,
    starts_with_video_keyframe: bool,
    container: InstantContainer,
    tracks: Vec<InstantTrackMetadata>,
    bytes: u64,
    sha256: Sha256Digest,
    identity: Sha256Digest,
}

impl fmt::Debug for InstantSegmentDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantSegmentDescriptor")
            .field("protocol_version", &self.protocol_version)
            .field("index", &self.index)
            .field("start_ns", &self.start_ns)
            .field("duration_ns", &self.duration_ns)
            .field("track_count", &self.tracks.len())
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl InstantSegmentDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: InstantSessionId,
        index: u32,
        start_ns: u64,
        duration_ns: u64,
        starts_with_video_keyframe: bool,
        container: InstantContainer,
        tracks: Vec<InstantTrackMetadata>,
        bytes: u64,
        sha256: Sha256Digest,
    ) -> Result<Self, InstantError> {
        if index >= MAX_INSTANT_SEGMENTS
            || duration_ns == 0
            || start_ns.checked_add(duration_ns).is_none()
            || bytes == 0
            || bytes > MAX_INSTANT_SEGMENT_BYTES
            || !starts_with_video_keyframe
            || tracks.is_empty()
            || tracks.len() > MAX_INSTANT_TRACKS
        {
            return Err(InstantError::InvalidSegment);
        }
        let mut numbers = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for track in &tracks {
            if !numbers.insert(track.track_number)
                || !roles.insert(track.role)
                || track.first_presentation_ns != start_ns
                || track.duration_ns != duration_ns
            {
                return Err(InstantError::TrackAlignmentMismatch);
            }
        }
        if !roles.contains(&InstantTrackRole::ScreenVideo) {
            return Err(InstantError::MissingScreenTrack);
        }
        let mut descriptor = Self {
            protocol_version: INSTANT_PROTOCOL_VERSION,
            session_id,
            index,
            start_ns,
            duration_ns,
            starts_with_video_keyframe,
            container,
            tracks,
            bytes,
            sha256,
            identity: strong_sha256(b"pending-segment-identity"),
        };
        descriptor.identity = descriptor.compute_identity();
        Ok(descriptor)
    }

    #[must_use]
    pub const fn session_id(&self) -> InstantSessionId {
        self.session_id
    }

    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    #[must_use]
    pub const fn start_ns(&self) -> u64 {
        self.start_ns
    }

    #[must_use]
    pub const fn duration_ns(&self) -> u64 {
        self.duration_ns
    }

    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    #[must_use]
    pub const fn sha256(&self) -> Sha256Digest {
        self.sha256
    }

    #[must_use]
    pub const fn identity(&self) -> Sha256Digest {
        self.identity
    }

    pub(crate) const fn starts_with_video_keyframe(&self) -> bool {
        self.starts_with_video_keyframe
    }

    pub(crate) const fn container(&self) -> InstantContainer {
        self.container
    }

    pub(crate) fn tracks(&self) -> &[InstantTrackMetadata] {
        &self.tracks
    }

    fn compute_identity(&self) -> Sha256Digest {
        let mut canonical = Vec::with_capacity(256);
        canonical.extend_from_slice(b"frame.instant.segment.v2\0");
        canonical.extend_from_slice(&self.protocol_version.to_be_bytes());
        canonical.extend_from_slice(&self.session_id.canonical_bytes());
        canonical.extend_from_slice(&self.index.to_be_bytes());
        canonical.extend_from_slice(&self.start_ns.to_be_bytes());
        canonical.extend_from_slice(&self.duration_ns.to_be_bytes());
        canonical.push(u8::from(self.starts_with_video_keyframe));
        canonical.push(match self.container {
            InstantContainer::FragmentedMp4Cmaf => 1,
        });
        canonical.extend_from_slice(&self.bytes.to_be_bytes());
        canonical.extend_from_slice(&self.sha256.canonical_bytes());
        canonical.extend_from_slice(&(self.tracks.len() as u32).to_be_bytes());
        for track in &self.tracks {
            canonical.extend_from_slice(&track.track_number.to_be_bytes());
            canonical.push(track_role_tag(track.role));
            canonical.push(codec_tag(track.codec));
            canonical.extend_from_slice(&track.timescale.to_be_bytes());
            canonical.extend_from_slice(&track.sample_count.to_be_bytes());
            canonical.extend_from_slice(&track.first_presentation_ns.to_be_bytes());
            canonical.extend_from_slice(&track.duration_ns.to_be_bytes());
        }
        strong_sha256(&canonical)
    }
}

fn track_role_tag(role: InstantTrackRole) -> u8 {
    match role {
        InstantTrackRole::ScreenVideo => 1,
        InstantTrackRole::CameraVideo => 2,
        InstantTrackRole::MixedAudio => 3,
    }
}

fn codec_tag(codec: InstantCodec) -> u8 {
    match codec {
        InstantCodec::H264Avc => 1,
        InstantCodec::AacLowComplexity => 2,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct InstantManifest {
    protocol_version: u16,
    manifest_version: u16,
    session_id: InstantSessionId,
    segment_count: u32,
    total_bytes: u64,
    duration_ns: u64,
    segment_identities: Vec<Sha256Digest>,
    digest: Sha256Digest,
}

impl fmt::Debug for InstantManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantManifest")
            .field("protocol_version", &self.protocol_version)
            .field("manifest_version", &self.manifest_version)
            .field("segment_count", &self.segment_count)
            .field("total_bytes", &self.total_bytes)
            .field("duration_ns", &self.duration_ns)
            .finish_non_exhaustive()
    }
}

impl InstantManifest {
    pub fn from_segments(
        session_id: InstantSessionId,
        segments: impl IntoIterator<Item = InstantSegmentDescriptor>,
    ) -> Result<Self, InstantError> {
        let mut ordered = BTreeMap::new();
        for segment in segments {
            if segment.session_id != session_id {
                return Err(InstantError::SessionBindingMismatch);
            }
            let index = segment.index;
            if ordered.insert(index, segment).is_some() {
                return Err(InstantError::SegmentConflict(index));
            }
        }
        if ordered.is_empty() {
            return Err(InstantError::NoSegments);
        }
        let mut expected_index = 0_u32;
        let mut expected_start = 0_u64;
        let mut total_bytes = 0_u64;
        let mut identities = Vec::with_capacity(ordered.len());
        for (index, segment) in ordered {
            if index != expected_index || segment.start_ns != expected_start {
                return Err(InstantError::SegmentContinuity {
                    expected_index,
                    found_index: index,
                });
            }
            expected_index = expected_index
                .checked_add(1)
                .ok_or(InstantError::JournalCorrupt)?;
            expected_start = expected_start
                .checked_add(segment.duration_ns)
                .ok_or(InstantError::JournalCorrupt)?;
            total_bytes = total_bytes
                .checked_add(segment.bytes)
                .ok_or(InstantError::JournalCorrupt)?;
            identities.push(segment.identity);
        }
        let mut canonical = Vec::with_capacity(64 + identities.len() * 32);
        canonical.extend_from_slice(b"frame.instant.manifest.v1\0");
        canonical.extend_from_slice(&INSTANT_PROTOCOL_VERSION.to_be_bytes());
        canonical.extend_from_slice(&INSTANT_MANIFEST_VERSION.to_be_bytes());
        canonical.extend_from_slice(&session_id.canonical_bytes());
        canonical.extend_from_slice(&expected_index.to_be_bytes());
        canonical.extend_from_slice(&total_bytes.to_be_bytes());
        canonical.extend_from_slice(&expected_start.to_be_bytes());
        for identity in &identities {
            canonical.extend_from_slice(&identity.canonical_bytes());
        }
        Ok(Self {
            protocol_version: INSTANT_PROTOCOL_VERSION,
            manifest_version: INSTANT_MANIFEST_VERSION,
            session_id,
            segment_count: expected_index,
            total_bytes,
            duration_ns: expected_start,
            segment_identities: identities,
            digest: strong_sha256(&canonical),
        })
    }

    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    #[must_use]
    pub const fn segment_count(&self) -> u32 {
        self.segment_count
    }

    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub const fn duration_ns(&self) -> u64 {
        self.duration_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantVideoCaps {
    pub width: u16,
    pub height: u16,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantAudioCaps {
    pub sample_rate: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantPipelineCapabilities {
    pub splitmuxsink: bool,
    pub mp4mux_fragmented: bool,
    pub h264_avc_encoder: bool,
    pub aac_lc_encoder: bool,
    pub force_key_unit: bool,
    pub exact_split_running_time: bool,
    pub aligned_audio_fragments: bool,
    pub media_distribution_master: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantPipelineRequest {
    pub video: InstantVideoCaps,
    pub audio: Option<InstantAudioCaps>,
    pub segment_duration_ns: u64,
    pub max_split_slip_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantPipelineNode {
    ScreenAppSrc,
    BoundedVideoQueue,
    VideoConvert,
    ExactVideoCaps,
    H264AvcEncoder,
    H264Parser,
    AudioAppSrc,
    BoundedAudioQueue,
    AudioConvert,
    AudioResample,
    ExactAudioCaps,
    AacLcEncoder,
    AacParser,
    FragmentedMp4Muxer,
    SplitMuxSink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantPipelineSpec {
    contract_version: u16,
    request: InstantPipelineRequest,
    nodes: Vec<InstantPipelineNode>,
    keyframe_at_every_split: bool,
    tracks_split_on_same_running_time: bool,
    distribution_master_eligible: bool,
}

impl InstantPipelineSpec {
    pub fn negotiate(
        capabilities: InstantPipelineCapabilities,
        request: InstantPipelineRequest,
    ) -> Result<Self, InstantError> {
        if request.video.width == 0
            || request.video.height == 0
            || request.video.frame_rate_numerator == 0
            || request.video.frame_rate_denominator == 0
            || request.segment_duration_ns < 250_000_000
            || request.segment_duration_ns > 30_000_000_000
            || request.max_split_slip_ns > request.segment_duration_ns / 4
        {
            return Err(InstantError::InvalidPipelineRequest);
        }
        if let Some(audio) = request.audio
            && (audio.sample_rate != 48_000 || audio.channels != 2)
        {
            return Err(InstantError::InvalidPipelineRequest);
        }
        if !capabilities.splitmuxsink
            || !capabilities.mp4mux_fragmented
            || !capabilities.h264_avc_encoder
            || !capabilities.force_key_unit
            || !capabilities.exact_split_running_time
            || !capabilities.media_distribution_master
            || (request.audio.is_some()
                && (!capabilities.aac_lc_encoder || !capabilities.aligned_audio_fragments))
        {
            return Err(InstantError::DistributionMasterUnavailable);
        }
        let mut nodes = vec![
            InstantPipelineNode::ScreenAppSrc,
            InstantPipelineNode::BoundedVideoQueue,
            InstantPipelineNode::VideoConvert,
            InstantPipelineNode::ExactVideoCaps,
            InstantPipelineNode::H264AvcEncoder,
            InstantPipelineNode::H264Parser,
        ];
        if request.audio.is_some() {
            nodes.extend_from_slice(&[
                InstantPipelineNode::AudioAppSrc,
                InstantPipelineNode::BoundedAudioQueue,
                InstantPipelineNode::AudioConvert,
                InstantPipelineNode::AudioResample,
                InstantPipelineNode::ExactAudioCaps,
                InstantPipelineNode::AacLcEncoder,
                InstantPipelineNode::AacParser,
            ]);
        }
        nodes.extend_from_slice(&[
            InstantPipelineNode::FragmentedMp4Muxer,
            InstantPipelineNode::SplitMuxSink,
        ]);
        Ok(Self {
            contract_version: INSTANT_PROTOCOL_VERSION,
            request,
            nodes,
            keyframe_at_every_split: true,
            tracks_split_on_same_running_time: true,
            distribution_master_eligible: true,
        })
    }

    #[must_use]
    pub const fn distribution_master_eligible(&self) -> bool {
        self.distribution_master_eligible
    }

    #[must_use]
    pub fn nodes(&self) -> &[InstantPipelineNode] {
        &self.nodes
    }
}

/// A one-owner pull body. Implementations must return non-empty chunks, EOF
/// exactly once, and release native resources from `cancel` and `Drop`.
pub trait InstantSegmentPayload: fmt::Debug {
    fn declared_len(&self) -> u64;
    fn pull(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, InstantError>;
    fn cancel(&mut self);
}

pub struct ValidatedSegmentPayload {
    inner: Box<dyn InstantSegmentPayload>,
    expected_len: u64,
    expected_sha256: Sha256Digest,
    observed_len: u64,
    hasher: Sha256State,
    max_chunk_bytes: usize,
    terminal: bool,
}

impl fmt::Debug for ValidatedSegmentPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedSegmentPayload")
            .field("expected_len", &self.expected_len)
            .field("observed_len", &self.observed_len)
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl ValidatedSegmentPayload {
    pub fn new(
        inner: Box<dyn InstantSegmentPayload>,
        descriptor: &InstantSegmentDescriptor,
        max_chunk_bytes: usize,
    ) -> Result<Self, InstantError> {
        if inner.declared_len() != descriptor.bytes
            || max_chunk_bytes == 0
            || max_chunk_bytes > MAX_INSTANT_PAYLOAD_CHUNK_BYTES
        {
            return Err(InstantError::PayloadLengthMismatch);
        }
        Ok(Self {
            inner,
            expected_len: descriptor.bytes,
            expected_sha256: descriptor.sha256,
            observed_len: 0,
            hasher: Sha256State::new(),
            max_chunk_bytes,
            terminal: false,
        })
    }

    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, InstantError> {
        if self.terminal {
            return Ok(None);
        }
        let next = self.inner.pull(self.max_chunk_bytes)?;
        match next {
            Some(chunk) => {
                if chunk.is_empty() || chunk.len() > self.max_chunk_bytes {
                    return Err(InstantError::InvalidPayloadChunk);
                }
                self.observed_len = self
                    .observed_len
                    .checked_add(chunk.len() as u64)
                    .ok_or(InstantError::PayloadLengthMismatch)?;
                if self.observed_len > self.expected_len {
                    return Err(InstantError::PayloadLengthMismatch);
                }
                self.hasher.update(&chunk);
                Ok(Some(chunk))
            }
            None => {
                if self.observed_len != self.expected_len {
                    return Err(InstantError::PayloadLengthMismatch);
                }
                let hasher = std::mem::replace(&mut self.hasher, Sha256State::new());
                if hasher.finalize() != self.expected_sha256 {
                    return Err(InstantError::PayloadChecksumMismatch);
                }
                self.terminal = true;
                Ok(None)
            }
        }
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.terminal
    }

    pub fn cancel(&mut self) {
        if !self.terminal {
            self.inner.cancel();
            self.terminal = true;
        }
    }
}

impl Drop for ValidatedSegmentPayload {
    fn drop(&mut self) {
        if !self.terminal {
            self.inner.cancel();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpoolAeadAlgorithm {
    Aes256Gcm,
    XChaCha20Poly1305,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpoolProtectionCapability {
    EncryptedAndAuthenticated {
        algorithm: SpoolAeadAlgorithm,
        atomic_replace: bool,
        private_permissions: bool,
    },
    PrivateButUnencrypted,
    Unavailable,
}

impl SpoolProtectionCapability {
    pub fn require_instant_safe(self) -> Result<SpoolAeadAlgorithm, InstantError> {
        match self {
            Self::EncryptedAndAuthenticated {
                algorithm,
                atomic_replace: true,
                private_permissions: true,
            } => Ok(algorithm),
            _ => Err(InstantError::SecureSpoolUnavailable),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolQuotaPolicy {
    pub max_retained_bytes: u64,
    pub max_reserved_bytes: u64,
    pub max_segment_bytes: u64,
}

impl SpoolQuotaPolicy {
    pub fn validate(self) -> Result<Self, InstantError> {
        if self.max_retained_bytes == 0
            || self.max_reserved_bytes == 0
            || self.max_segment_bytes == 0
            || self.max_segment_bytes > self.max_retained_bytes
            || self.max_reserved_bytes > self.max_retained_bytes
            || self.max_segment_bytes > MAX_INSTANT_SEGMENT_BYTES
        {
            return Err(InstantError::InvalidSpoolQuota);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SpoolWriteClaim {
    pub session_id: InstantSessionId,
    pub operation_id: InstantOperationId,
    pub segment_index: u32,
    pub segment_identity: Sha256Digest,
    pub bytes: u64,
}

impl fmt::Debug for SpoolWriteClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpoolWriteClaim")
            .field("segment_index", &self.segment_index)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolCommitReceipt {
    pub segment_index: u32,
    pub segment_identity: Sha256Digest,
    pub bytes: u64,
    pub ciphertext_integrity: Sha256Digest,
    pub durable: bool,
}

pub trait SpoolReservationLease: fmt::Debug {
    fn write(&mut self, chunk: &[u8]) -> Result<(), InstantError>;
    fn commit(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<SpoolCommitReceipt, InstantError>;
    fn abort(&mut self);
}

struct SpoolReservation {
    inner: Box<dyn SpoolReservationLease>,
    terminal: bool,
}

impl SpoolReservation {
    fn new(inner: Box<dyn SpoolReservationLease>) -> Self {
        Self {
            inner,
            terminal: false,
        }
    }

    fn write(&mut self, chunk: &[u8]) -> Result<(), InstantError> {
        self.inner.write(chunk)
    }

    fn commit(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<SpoolCommitReceipt, InstantError> {
        let receipt = self.inner.commit(descriptor)?;
        self.terminal = true;
        Ok(receipt)
    }
}

impl Drop for SpoolReservation {
    fn drop(&mut self) {
        if !self.terminal {
            self.inner.abort();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredSpoolEntry {
    pub descriptor: InstantSegmentDescriptor,
    pub commit_receipt: SpoolCommitReceipt,
    pub committed: bool,
}

pub trait PrivateSpoolPort: fmt::Debug {
    fn protection(&self) -> SpoolProtectionCapability;
    fn acquire_runtime_key(
        &mut self,
        session_id: InstantSessionId,
    ) -> Result<RuntimeSpoolKeyHandle, InstantError>;
    fn key_marker(&self, key: &RuntimeSpoolKeyHandle) -> Sha256Digest;
    fn reserve(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        claim: SpoolWriteClaim,
    ) -> Result<Box<dyn SpoolReservationLease>, InstantError>;
    fn open(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<Box<dyn InstantSegmentPayload>, InstantError>;
    fn recover(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
    ) -> Result<Vec<RecoveredSpoolEntry>, InstantError>;
    fn evict(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
        segment_identity: Sha256Digest,
    ) -> Result<(), InstantError>;
    fn wipe_session(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
    ) -> Result<(), InstantError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpoolEvictionPolicy {
    AfterVerifiedRemotePart,
    AfterFinalObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDurabilityProof {
    part_receipt: InstantPartReceipt,
    final_object: Option<InstantMultipartCompleteReceipt>,
}

impl RemoteDurabilityProof {
    pub fn with_final_object(
        mut self,
        complete: InstantMultipartCompleteReceipt,
    ) -> Result<Self, InstantError> {
        if complete.session_id != self.part_receipt.session_id
            || complete.upload_id != self.part_receipt.upload_id
            || complete.upload_generation < self.part_receipt.upload_generation
            || complete.object.instant_manifest != complete.manifest_digest
        {
            return Err(InstantError::RemoteDurabilityUnproven);
        }
        self.final_object = Some(complete);
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpoolRecoveryAction {
    JournalOrphanCommit {
        descriptor: InstantSegmentDescriptor,
        receipt: SpoolCommitReceipt,
    },
    JournalVerifiedEviction {
        index: u32,
        proof: RemoteDurabilityProof,
    },
    RemoveAlreadyJournaledEviction {
        descriptor: InstantSegmentDescriptor,
        proof: RemoteDurabilityProof,
    },
    WipeTombstonedSession,
}

#[derive(Debug, Clone)]
struct RetainedSpoolRecord {
    descriptor: InstantSegmentDescriptor,
    receipt: SpoolCommitReceipt,
}

pub struct InstantSpool<P: PrivateSpoolPort> {
    port: P,
    session_id: InstantSessionId,
    key: RuntimeSpoolKeyHandle,
    key_marker: Sha256Digest,
    quota: SpoolQuotaPolicy,
    retained: BTreeMap<u32, RetainedSpoolRecord>,
    retained_bytes: u64,
}

impl<P: PrivateSpoolPort> fmt::Debug for InstantSpool<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantSpool")
            .field("retained_segments", &self.retained.len())
            .field("retained_bytes", &self.retained_bytes)
            .finish_non_exhaustive()
    }
}

impl<P: PrivateSpoolPort> InstantSpool<P> {
    pub fn open(
        mut port: P,
        session_id: InstantSessionId,
        quota: SpoolQuotaPolicy,
    ) -> Result<Self, InstantError> {
        port.protection().require_instant_safe()?;
        let quota = quota.validate()?;
        let key = port.acquire_runtime_key(session_id)?;
        let key_marker = port.key_marker(&key);
        if !key.matches(key_marker) {
            return Err(InstantError::SpoolKeyUnavailable);
        }
        Ok(Self {
            port,
            session_id,
            key,
            key_marker,
            quota,
            retained: BTreeMap::new(),
            retained_bytes: 0,
        })
    }

    pub fn recover(mut self) -> Result<Self, InstantError> {
        let entries = self.port.recover(&self.key, self.session_id)?;
        let mut retained = BTreeMap::new();
        let mut retained_bytes = 0_u64;
        for entry in entries {
            if !entry.committed
                || entry.descriptor.session_id != self.session_id
                || entry.descriptor.bytes == 0
                || entry.descriptor.bytes > self.quota.max_segment_bytes
                || entry.commit_receipt.segment_index != entry.descriptor.index
                || entry.commit_receipt.segment_identity != entry.descriptor.identity
                || entry.commit_receipt.bytes != entry.descriptor.bytes
                || !entry.commit_receipt.durable
                || retained
                    .insert(
                        entry.descriptor.index,
                        RetainedSpoolRecord {
                            descriptor: entry.descriptor.clone(),
                            receipt: entry.commit_receipt,
                        },
                    )
                    .is_some()
            {
                return Err(InstantError::SpoolCorrupt);
            }
            retained_bytes = retained_bytes
                .checked_add(entry.descriptor.bytes)
                .ok_or(InstantError::SpoolCorrupt)?;
        }
        if retained_bytes > self.quota.max_retained_bytes {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        self.retained = retained;
        self.retained_bytes = retained_bytes;
        Ok(self)
    }

    pub fn commit_segment(
        &mut self,
        operation_id: InstantOperationId,
        descriptor: &InstantSegmentDescriptor,
        source: Box<dyn InstantSegmentPayload>,
    ) -> Result<SpoolCommitReceipt, InstantError> {
        if descriptor.session_id != self.session_id {
            return Err(InstantError::SessionBindingMismatch);
        }
        if descriptor.bytes > self.quota.max_segment_bytes {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        if let Some(existing) = self.retained.get(&descriptor.index) {
            return if existing.descriptor == *descriptor {
                Err(InstantError::OperationAlreadyApplied)
            } else {
                Err(InstantError::SegmentConflict(descriptor.index))
            };
        }
        let next_retained = self
            .retained_bytes
            .checked_add(descriptor.bytes)
            .ok_or(InstantError::SpoolQuotaExceeded)?;
        if next_retained > self.quota.max_retained_bytes
            || descriptor.bytes > self.quota.max_reserved_bytes
        {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        let claim = SpoolWriteClaim {
            session_id: self.session_id,
            operation_id,
            segment_index: descriptor.index,
            segment_identity: descriptor.identity,
            bytes: descriptor.bytes,
        };
        let mut reservation = SpoolReservation::new(self.port.reserve(&self.key, claim)?);
        let mut payload =
            ValidatedSegmentPayload::new(source, descriptor, MAX_INSTANT_PAYLOAD_CHUNK_BYTES)?;
        while let Some(chunk) = payload.next_chunk()? {
            reservation.write(&chunk)?;
        }
        let receipt = reservation.commit(descriptor)?;
        if !receipt.durable
            || receipt.segment_index != descriptor.index
            || receipt.segment_identity != descriptor.identity
            || receipt.bytes != descriptor.bytes
        {
            return Err(InstantError::InvalidSpoolReceipt);
        }
        self.retained.insert(
            descriptor.index,
            RetainedSpoolRecord {
                descriptor: descriptor.clone(),
                receipt,
            },
        );
        self.retained_bytes = next_retained;
        Ok(receipt)
    }

    pub fn open_upload(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<ValidatedSegmentPayload, InstantError> {
        if self
            .retained
            .get(&descriptor.index)
            .is_none_or(|record| record.descriptor != *descriptor)
        {
            return Err(InstantError::SpoolEntryMissing);
        }
        let body = self.port.open(&self.key, self.session_id, descriptor)?;
        ValidatedSegmentPayload::new(body, descriptor, MAX_INSTANT_PAYLOAD_CHUNK_BYTES)
    }

    pub fn evict(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
        proof: RemoteDurabilityProof,
        policy: SpoolEvictionPolicy,
    ) -> Result<bool, InstantError> {
        let receipt = proof.part_receipt;
        if receipt.session_id != descriptor.session_id
            || receipt.part_number != descriptor.index.saturating_add(1)
            || receipt.segment_identity != descriptor.identity
            || receipt.bytes != descriptor.bytes
            || receipt.sha256 != descriptor.sha256
            || (policy == SpoolEvictionPolicy::AfterFinalObject && proof.final_object.is_none())
        {
            return Err(InstantError::RemoteDurabilityUnproven);
        }
        let Some(record) = self.retained.get(&descriptor.index).cloned() else {
            return Ok(false);
        };
        if record.descriptor != *descriptor {
            return Err(InstantError::SpoolCorrupt);
        }
        self.port
            .evict(&self.key, self.session_id, descriptor.identity)?;
        self.retained.remove(&descriptor.index);
        self.retained_bytes = self
            .retained_bytes
            .checked_sub(record.descriptor.bytes)
            .ok_or(InstantError::SpoolCorrupt)?;
        Ok(true)
    }

    pub fn wipe(mut self) -> Result<(), InstantError> {
        self.port.wipe_session(&self.key, self.session_id)
    }

    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    #[must_use]
    pub const fn key_marker(&self) -> Sha256Digest {
        self.key_marker
    }

    /// Compares recovered durable files with the authoritative journal. The
    /// returned actions close only crash windows whose postconditions are
    /// already provable; an unverified missing file is corruption.
    pub fn reconcile_journal(
        &self,
        journal: &InstantJournalSnapshot,
    ) -> Result<Vec<SpoolRecoveryAction>, InstantError> {
        if journal.session_id != self.session_id {
            return Err(InstantError::SessionBindingMismatch);
        }
        if journal.state == InstantJournalState::Tombstoned {
            return Ok(if self.retained.is_empty() {
                Vec::new()
            } else {
                vec![SpoolRecoveryAction::WipeTombstonedSession]
            });
        }
        let mut actions = Vec::new();
        for (index, segment) in &journal.segments {
            match (segment.spool_present, self.retained.get(index)) {
                (true, Some(record))
                    if record.descriptor == segment.descriptor
                        && record.receipt == segment.spool_receipt => {}
                (true, Some(_)) => return Err(InstantError::SpoolCorrupt),
                (true, None) => {
                    let SegmentUploadJournalState::Verified(receipt) = segment.upload else {
                        return Err(InstantError::SpoolCorrupt);
                    };
                    actions.push(SpoolRecoveryAction::JournalVerifiedEviction {
                        index: *index,
                        proof: receipt.durability_proof(),
                    });
                }
                (false, Some(record))
                    if record.descriptor == segment.descriptor
                        && record.receipt == segment.spool_receipt =>
                {
                    let SegmentUploadJournalState::Verified(receipt) = segment.upload else {
                        return Err(InstantError::SpoolCorrupt);
                    };
                    actions.push(SpoolRecoveryAction::RemoveAlreadyJournaledEviction {
                        descriptor: record.descriptor.clone(),
                        proof: receipt.durability_proof(),
                    });
                }
                (false, Some(_)) => return Err(InstantError::SpoolCorrupt),
                (false, None) => {}
            }
        }

        let mut expected_index = journal.segments.len() as u32;
        let mut expected_start = journal.segments.values().next_back().map_or(0, |segment| {
            segment
                .descriptor
                .start_ns
                .saturating_add(segment.descriptor.duration_ns)
        });
        for (index, record) in self.retained.range(expected_index..) {
            if *index != expected_index || record.descriptor.start_ns != expected_start {
                return Err(InstantError::SpoolCorrupt);
            }
            actions.push(SpoolRecoveryAction::JournalOrphanCommit {
                descriptor: record.descriptor.clone(),
                receipt: record.receipt,
            });
            expected_index = expected_index
                .checked_add(1)
                .ok_or(InstantError::SpoolCorrupt)?;
            expected_start = expected_start
                .checked_add(record.descriptor.duration_ns)
                .ok_or(InstantError::SpoolCorrupt)?;
        }
        Ok(actions)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstantFence(u64);

impl InstantFence {
    pub fn new(value: u64) -> Result<Self, InstantError> {
        if value == 0 {
            return Err(InstantError::InvalidFence);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for InstantFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InstantFence(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantJournalState {
    Created,
    CapturingOnline,
    CapturingOffline,
    Finalizing,
    Ready,
    Tombstoned,
    RecoverableFailure,
}

impl InstantJournalState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Ready | Self::Tombstoned)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadDeferReason {
    Offline,
    Throttled,
    ProviderUnavailable,
    UploadExpired,
    LostAcknowledgement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantPartWorkKind {
    Upload,
    Probe,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InstantMultipartBinding {
    pub session_id: InstantSessionId,
    pub upload_id: InstantUploadId,
    pub expires_at_ns: u64,
    pub generation: u64,
    pub minimum_part_bytes: u64,
    pub maximum_part_bytes: u64,
    pub maximum_parts: u32,
}

impl fmt::Debug for InstantMultipartBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantMultipartBinding")
            .field("expires_at_ns", &self.expires_at_ns)
            .field("generation", &self.generation)
            .field("minimum_part_bytes", &self.minimum_part_bytes)
            .field("maximum_part_bytes", &self.maximum_part_bytes)
            .field("maximum_parts", &self.maximum_parts)
            .finish_non_exhaustive()
    }
}

impl InstantMultipartBinding {
    pub fn validate(self) -> Result<Self, InstantError> {
        if self.expires_at_ns == 0
            || self.generation == 0
            || self.minimum_part_bytes == 0
            || self.maximum_part_bytes < self.minimum_part_bytes
            || self.maximum_part_bytes > MAX_INSTANT_SEGMENT_BYTES
            || self.maximum_parts == 0
            || self.maximum_parts > MAX_INSTANT_SEGMENTS
        {
            return Err(InstantError::InvalidMultipartBinding);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InstantPartReceipt {
    pub session_id: InstantSessionId,
    pub upload_id: InstantUploadId,
    pub upload_generation: u64,
    pub part_number: u32,
    pub bytes: u64,
    pub sha256: Sha256Digest,
    pub segment_identity: Sha256Digest,
    pub provider_receipt_digest: Sha256Digest,
}

impl fmt::Debug for InstantPartReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantPartReceipt")
            .field("upload_generation", &self.upload_generation)
            .field("part_number", &self.part_number)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl InstantPartReceipt {
    pub fn new(
        binding: InstantMultipartBinding,
        descriptor: &InstantSegmentDescriptor,
        provider_receipt_digest: Sha256Digest,
    ) -> Result<Self, InstantError> {
        let binding = binding
            .validate()
            .map_err(|_| InstantError::InvalidPartReceipt)?;
        let receipt = Self {
            session_id: binding.session_id,
            upload_id: binding.upload_id,
            upload_generation: binding.generation,
            part_number: descriptor
                .index
                .checked_add(1)
                .ok_or(InstantError::InvalidPartReceipt)?,
            bytes: descriptor.bytes,
            sha256: descriptor.sha256,
            segment_identity: descriptor.identity,
            provider_receipt_digest,
        };
        receipt.validate_for(binding, descriptor)
    }

    #[must_use]
    pub fn durability_proof(self) -> RemoteDurabilityProof {
        RemoteDurabilityProof {
            part_receipt: self,
            final_object: None,
        }
    }

    fn validate_for(
        self,
        binding: InstantMultipartBinding,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<Self, InstantError> {
        if self.session_id != binding.session_id
            || descriptor.session_id != binding.session_id
            || self.upload_id != binding.upload_id
            || self.upload_generation != binding.generation
            || self.part_number != descriptor.index.saturating_add(1)
            || self.part_number == 0
            || self.part_number > binding.maximum_parts
            || self.bytes != descriptor.bytes
            || self.bytes > binding.maximum_part_bytes
            || self.sha256 != descriptor.sha256
            || self.segment_identity != descriptor.identity
        {
            return Err(InstantError::InvalidPartReceipt);
        }
        Ok(self)
    }

    fn validate_persisted_for(
        self,
        binding: InstantMultipartBinding,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<Self, InstantError> {
        if self.upload_generation == 0 || self.upload_generation > binding.generation {
            return Err(InstantError::InvalidPartReceipt);
        }
        self.validate_for(
            InstantMultipartBinding {
                generation: self.upload_generation,
                ..binding
            },
            descriptor,
        )
    }

    fn digest(self) -> Sha256Digest {
        let mut canonical = Vec::with_capacity(160);
        canonical.extend_from_slice(b"frame.instant.part-receipt.v1\0");
        canonical.extend_from_slice(&self.session_id.canonical_bytes());
        canonical.extend_from_slice(&self.upload_id.canonical_bytes());
        canonical.extend_from_slice(&self.upload_generation.to_be_bytes());
        canonical.extend_from_slice(&self.part_number.to_be_bytes());
        canonical.extend_from_slice(&self.bytes.to_be_bytes());
        canonical.extend_from_slice(&self.sha256.canonical_bytes());
        canonical.extend_from_slice(&self.segment_identity.canonical_bytes());
        canonical.extend_from_slice(&self.provider_receipt_digest.canonical_bytes());
        strong_sha256(&canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentUploadJournalState {
    Queued {
        attempt: u16,
        eligible_at_ns: u64,
    },
    InFlight {
        worker_id: InstantWorkerId,
        fence: InstantFence,
        work_kind: InstantPartWorkKind,
        attempt: u16,
        lease_until_ns: u64,
    },
    Deferred {
        attempt: u16,
        eligible_at_ns: u64,
        reason: UploadDeferReason,
    },
    ProbeRequired {
        attempt: u16,
        eligible_at_ns: u64,
    },
    Verified(InstantPartReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantJournalSegment {
    descriptor: InstantSegmentDescriptor,
    spool_receipt: SpoolCommitReceipt,
    spool_present: bool,
    upload: SegmentUploadJournalState,
}

impl InstantJournalSegment {
    #[must_use]
    pub const fn descriptor(&self) -> &InstantSegmentDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn upload(&self) -> &SegmentUploadJournalState {
        &self.upload
    }

    #[must_use]
    pub const fn spool_present(&self) -> bool {
        self.spool_present
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantOperationKind {
    FenceAcquire,
    Begin,
    Connectivity,
    SegmentCommit,
    MultipartBind,
    MultipartRenew,
    PartClaim,
    PartDefer,
    PartProbe,
    PartVerify,
    SpoolEvict,
    FinalizeRequest,
    MultipartComplete,
    FinalizeDispatch,
    Publish,
    Callback,
    Cancel,
    RecoverableFailure,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InstantOperationReceipt {
    pub operation_id: InstantOperationId,
    pub kind: InstantOperationKind,
    pub committed_revision: u64,
    pub command_digest: Sha256Digest,
    pub outcome_digest: Sha256Digest,
}

impl fmt::Debug for InstantOperationReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantOperationReceipt")
            .field("kind", &self.kind)
            .field("committed_revision", &self.committed_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InstantObjectManifest {
    pub object_id: InstantObjectId,
    pub object_version: Sha256Digest,
    pub instant_manifest: Sha256Digest,
    pub ordered_parts_digest: Sha256Digest,
    pub bytes: u64,
}

impl fmt::Debug for InstantObjectManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantObjectManifest")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InstantMultipartCompleteReceipt {
    pub session_id: InstantSessionId,
    pub upload_id: InstantUploadId,
    pub upload_generation: u64,
    pub manifest_digest: Sha256Digest,
    pub ordered_parts_digest: Sha256Digest,
    pub object: InstantObjectManifest,
}

impl InstantMultipartCompleteReceipt {
    pub fn new(
        binding: InstantMultipartBinding,
        manifest: &InstantManifest,
        ordered_parts: &[InstantPartReceipt],
        object_id: InstantObjectId,
        object_version: Sha256Digest,
    ) -> Result<Self, InstantError> {
        let binding = binding
            .validate()
            .map_err(|_| InstantError::InvalidMultipartCompleteReceipt)?;
        if manifest.session_id != binding.session_id
            || ordered_parts.len() != manifest.segment_count as usize
        {
            return Err(InstantError::InvalidMultipartCompleteReceipt);
        }
        let mut bytes = Vec::with_capacity(64 + ordered_parts.len() * 32);
        bytes.extend_from_slice(b"frame.instant.ordered-parts.v1\0");
        for (index, receipt) in ordered_parts.iter().copied().enumerate() {
            if receipt.session_id != binding.session_id
                || receipt.upload_id != binding.upload_id
                || receipt.upload_generation == 0
                || receipt.upload_generation > binding.generation
                || receipt.part_number != (index as u32).saturating_add(1)
                || receipt.bytes > binding.maximum_part_bytes
                || (index + 1 < ordered_parts.len() && receipt.bytes < binding.minimum_part_bytes)
            {
                return Err(InstantError::InvalidMultipartCompleteReceipt);
            }
            bytes.extend_from_slice(&receipt.digest().canonical_bytes());
        }
        let ordered_parts_digest = strong_sha256(&bytes);
        let object = InstantObjectManifest {
            object_id,
            object_version,
            instant_manifest: manifest.digest,
            ordered_parts_digest,
            bytes: manifest.total_bytes,
        };
        Ok(Self {
            session_id: binding.session_id,
            upload_id: binding.upload_id,
            upload_generation: binding.generation,
            manifest_digest: manifest.digest,
            ordered_parts_digest,
            object,
        })
    }
}

impl fmt::Debug for InstantMultipartCompleteReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantMultipartCompleteReceipt")
            .field("upload_generation", &self.upload_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PlayableMasterIdentity {
    pub object_id: InstantObjectId,
    pub immutable_digest: Sha256Digest,
    pub distribution_eligible: bool,
}

impl fmt::Debug for PlayableMasterIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlayableMasterIdentity")
            .field("distribution_eligible", &self.distribution_eligible)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InstantFinalizeReceipt {
    pub request_digest: Sha256Digest,
    pub publication_id: InstantPublicationId,
    pub job_id: InstantJobId,
    pub job_generation: u64,
    pub object: InstantObjectManifest,
    pub playable_master: PlayableMasterIdentity,
}

impl InstantFinalizeReceipt {
    pub fn new(
        request: &InstantFinalizeRequest,
        publication_id: InstantPublicationId,
    ) -> Result<Self, InstantError> {
        let receipt = Self {
            request_digest: request.digest,
            publication_id,
            job_id: request.job_id,
            job_generation: request.job_generation,
            object: request.multipart.object,
            playable_master: PlayableMasterIdentity {
                object_id: request.multipart.object.object_id,
                immutable_digest: request.multipart.object.object_version,
                distribution_eligible: true,
            },
        };
        validate_finalize_receipt(request, receipt)?;
        Ok(receipt)
    }
}

impl fmt::Debug for InstantFinalizeReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeReceipt")
            .field("job_generation", &self.job_generation)
            .field(
                "distribution_eligible",
                &self.playable_master.distribution_eligible,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstantJournalCommand {
    AdvanceFence {
        next_fence: InstantFence,
        worker_id: InstantWorkerId,
    },
    Begin {
        network_available: bool,
    },
    Connectivity {
        network_available: bool,
    },
    CommitSegment {
        descriptor: InstantSegmentDescriptor,
        spool_receipt: SpoolCommitReceipt,
    },
    BindMultipart {
        binding: InstantMultipartBinding,
    },
    RenewMultipart {
        binding: InstantMultipartBinding,
    },
    ClaimPart {
        index: u32,
        worker_id: InstantWorkerId,
        work_kind: InstantPartWorkKind,
        attempt: u16,
        lease_until_ns: u64,
    },
    DeferPart {
        index: u32,
        attempt: u16,
        eligible_at_ns: u64,
        reason: UploadDeferReason,
    },
    RequirePartProbe {
        index: u32,
        attempt: u16,
        eligible_at_ns: u64,
    },
    VerifyPart {
        index: u32,
        receipt: InstantPartReceipt,
    },
    EvictSpool {
        index: u32,
        proof: RemoteDurabilityProof,
        policy: SpoolEvictionPolicy,
    },
    RequestFinalize {
        job_generation: u64,
    },
    CompleteMultipart {
        receipt: InstantMultipartCompleteReceipt,
    },
    SealFinalizeRequest,
    Publish {
        receipt: InstantFinalizeReceipt,
    },
    ApplyCallback {
        receipt: InstantFinalizeReceipt,
    },
    Cancel,
    MarkRecoverableFailure,
}

impl InstantJournalCommand {
    fn kind(&self) -> InstantOperationKind {
        match self {
            Self::AdvanceFence { .. } => InstantOperationKind::FenceAcquire,
            Self::Begin { .. } => InstantOperationKind::Begin,
            Self::Connectivity { .. } => InstantOperationKind::Connectivity,
            Self::CommitSegment { .. } => InstantOperationKind::SegmentCommit,
            Self::BindMultipart { .. } => InstantOperationKind::MultipartBind,
            Self::RenewMultipart { .. } => InstantOperationKind::MultipartRenew,
            Self::ClaimPart { .. } => InstantOperationKind::PartClaim,
            Self::DeferPart { .. } => InstantOperationKind::PartDefer,
            Self::RequirePartProbe { .. } => InstantOperationKind::PartProbe,
            Self::VerifyPart { .. } => InstantOperationKind::PartVerify,
            Self::EvictSpool { .. } => InstantOperationKind::SpoolEvict,
            Self::RequestFinalize { .. } => InstantOperationKind::FinalizeRequest,
            Self::CompleteMultipart { .. } => InstantOperationKind::MultipartComplete,
            Self::SealFinalizeRequest => InstantOperationKind::FinalizeDispatch,
            Self::Publish { .. } => InstantOperationKind::Publish,
            Self::ApplyCallback { .. } => InstantOperationKind::Callback,
            Self::Cancel => InstantOperationKind::Cancel,
            Self::MarkRecoverableFailure => InstantOperationKind::RecoverableFailure,
        }
    }

    fn digest(&self) -> Sha256Digest {
        let mut bytes = Vec::with_capacity(320);
        bytes.extend_from_slice(b"frame.instant.journal-command.v1\0");
        bytes.push(operation_kind_tag(self.kind()));
        match self {
            Self::AdvanceFence {
                next_fence,
                worker_id,
            } => {
                bytes.extend_from_slice(&next_fence.get().to_be_bytes());
                bytes.extend_from_slice(&worker_id.canonical_bytes());
            }
            Self::Begin { network_available } | Self::Connectivity { network_available } => {
                bytes.push(u8::from(*network_available));
            }
            Self::CommitSegment {
                descriptor,
                spool_receipt,
            } => {
                bytes.extend_from_slice(&descriptor.identity.canonical_bytes());
                append_spool_receipt(&mut bytes, *spool_receipt);
            }
            Self::BindMultipart { binding } | Self::RenewMultipart { binding } => {
                append_multipart_binding(&mut bytes, *binding);
            }
            Self::ClaimPart {
                index,
                worker_id,
                work_kind,
                attempt,
                lease_until_ns,
            } => {
                bytes.extend_from_slice(&index.to_be_bytes());
                bytes.extend_from_slice(&worker_id.canonical_bytes());
                bytes.push(match work_kind {
                    InstantPartWorkKind::Upload => 1,
                    InstantPartWorkKind::Probe => 2,
                });
                bytes.extend_from_slice(&attempt.to_be_bytes());
                bytes.extend_from_slice(&lease_until_ns.to_be_bytes());
            }
            Self::DeferPart {
                index,
                attempt,
                eligible_at_ns,
                reason,
            } => {
                bytes.extend_from_slice(&index.to_be_bytes());
                bytes.extend_from_slice(&attempt.to_be_bytes());
                bytes.extend_from_slice(&eligible_at_ns.to_be_bytes());
                bytes.push(defer_reason_tag(*reason));
            }
            Self::RequirePartProbe {
                index,
                attempt,
                eligible_at_ns,
            } => {
                bytes.extend_from_slice(&index.to_be_bytes());
                bytes.extend_from_slice(&attempt.to_be_bytes());
                bytes.extend_from_slice(&eligible_at_ns.to_be_bytes());
            }
            Self::VerifyPart { index, receipt } => {
                bytes.extend_from_slice(&index.to_be_bytes());
                bytes.extend_from_slice(&receipt.digest().canonical_bytes());
            }
            Self::EvictSpool {
                index,
                proof,
                policy,
            } => {
                bytes.extend_from_slice(&index.to_be_bytes());
                bytes.extend_from_slice(&proof.part_receipt.digest().canonical_bytes());
                if let Some(complete) = proof.final_object {
                    append_complete_receipt(&mut bytes, complete);
                } else {
                    bytes.extend_from_slice(&strong_sha256(b"no-final-object").canonical_bytes());
                }
                bytes.push(match policy {
                    SpoolEvictionPolicy::AfterVerifiedRemotePart => 1,
                    SpoolEvictionPolicy::AfterFinalObject => 2,
                });
            }
            Self::CompleteMultipart { receipt } => {
                append_complete_receipt(&mut bytes, *receipt);
            }
            Self::Publish { receipt } | Self::ApplyCallback { receipt } => {
                append_finalize_receipt(&mut bytes, *receipt);
            }
            Self::RequestFinalize { job_generation } => {
                bytes.extend_from_slice(&job_generation.to_be_bytes());
            }
            Self::SealFinalizeRequest | Self::Cancel | Self::MarkRecoverableFailure => {}
        }
        strong_sha256(&bytes)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct InstantJournalSnapshot {
    protocol_version: u16,
    journal_version: u16,
    session_id: InstantSessionId,
    revision: u64,
    fence: InstantFence,
    state: InstantJournalState,
    segments: BTreeMap<u32, InstantJournalSegment>,
    multipart: Option<InstantMultipartBinding>,
    manifest: Option<InstantManifest>,
    multipart_complete: Option<InstantMultipartCompleteReceipt>,
    job_generation: Option<u64>,
    finalize_request: Option<InstantFinalizeRequest>,
    publication: Option<InstantFinalizeReceipt>,
    receipts: BTreeMap<InstantOperationId, InstantOperationReceipt>,
}

impl fmt::Debug for InstantJournalSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantJournalSnapshot")
            .field("protocol_version", &self.protocol_version)
            .field("journal_version", &self.journal_version)
            .field("revision", &self.revision)
            .field("state", &self.state)
            .field("segment_count", &self.segments.len())
            .field("multipart_bound", &self.multipart.is_some())
            .field("manifest_built", &self.manifest.is_some())
            .field("published", &self.publication.is_some())
            .finish_non_exhaustive()
    }
}

impl InstantJournalSnapshot {
    pub fn new(
        session_id: InstantSessionId,
        initial_fence: InstantFence,
    ) -> Result<Self, InstantError> {
        Ok(Self {
            protocol_version: INSTANT_PROTOCOL_VERSION,
            journal_version: INSTANT_JOURNAL_VERSION,
            session_id,
            revision: 1,
            fence: initial_fence,
            state: InstantJournalState::Created,
            segments: BTreeMap::new(),
            multipart: None,
            manifest: None,
            multipart_complete: None,
            job_generation: None,
            finalize_request: None,
            publication: None,
            receipts: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn session_id(&self) -> InstantSessionId {
        self.session_id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn fence(&self) -> InstantFence {
        self.fence
    }

    #[must_use]
    pub const fn state(&self) -> InstantJournalState {
        self.state
    }

    #[must_use]
    pub fn segments(&self) -> impl ExactSizeIterator<Item = &InstantJournalSegment> {
        self.segments.values()
    }

    #[must_use]
    pub const fn manifest(&self) -> Option<&InstantManifest> {
        self.manifest.as_ref()
    }

    #[must_use]
    pub const fn multipart(&self) -> Option<InstantMultipartBinding> {
        self.multipart
    }

    #[must_use]
    pub const fn multipart_complete(&self) -> Option<InstantMultipartCompleteReceipt> {
        self.multipart_complete
    }

    #[must_use]
    pub const fn publication(&self) -> Option<InstantFinalizeReceipt> {
        self.publication
    }

    #[must_use]
    pub fn operation_receipt(
        &self,
        operation_id: InstantOperationId,
    ) -> Option<InstantOperationReceipt> {
        self.receipts.get(&operation_id).copied()
    }

    pub fn transition(
        &self,
        expected_revision: u64,
        expected_fence: InstantFence,
        operation_id: InstantOperationId,
        command: InstantJournalCommand,
        now_ns: u64,
    ) -> Result<InstantJournalTransition, InstantError> {
        let command_digest = command.digest();
        if let Some(receipt) = self.receipts.get(&operation_id).copied() {
            if receipt.command_digest != command_digest || receipt.kind != command.kind() {
                return Err(InstantError::OperationKeyConflict);
            }
            return Ok(InstantJournalTransition {
                expected_revision: self.revision,
                expected_fence: self.fence,
                snapshot: self.clone(),
                receipt,
                requires_write: false,
            });
        }
        if self.revision != expected_revision {
            return Err(InstantError::StaleJournal);
        }
        if self.fence != expected_fence {
            return Err(InstantError::StaleFence);
        }
        if self.receipts.len() >= MAX_INSTANT_OPERATION_RECEIPTS {
            return Err(InstantError::OperationReceiptCapacityExceeded);
        }
        let mut next = self.clone();
        next.apply_command(&command, now_ns)?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(InstantError::JournalCorrupt)?;
        let outcome_digest = next.outcome_digest(command.kind());
        let receipt = InstantOperationReceipt {
            operation_id,
            kind: command.kind(),
            committed_revision: next.revision,
            command_digest,
            outcome_digest,
        };
        next.receipts.insert(operation_id, receipt);
        Ok(InstantJournalTransition {
            expected_revision,
            expected_fence,
            snapshot: next,
            receipt,
            requires_write: true,
        })
    }

    fn apply_command(
        &mut self,
        command: &InstantJournalCommand,
        now_ns: u64,
    ) -> Result<(), InstantError> {
        match command {
            InstantJournalCommand::AdvanceFence { next_fence, .. } => {
                if next_fence.get() <= self.fence.get() {
                    return Err(InstantError::StaleFence);
                }
                self.fence = *next_fence;
            }
            InstantJournalCommand::Begin { network_available } => {
                self.require_state(&[InstantJournalState::Created])?;
                self.state = if *network_available {
                    InstantJournalState::CapturingOnline
                } else {
                    InstantJournalState::CapturingOffline
                };
            }
            InstantJournalCommand::Connectivity { network_available } => {
                self.require_state(&[
                    InstantJournalState::CapturingOnline,
                    InstantJournalState::CapturingOffline,
                ])?;
                self.state = if *network_available {
                    InstantJournalState::CapturingOnline
                } else {
                    InstantJournalState::CapturingOffline
                };
            }
            InstantJournalCommand::CommitSegment {
                descriptor,
                spool_receipt,
            } => self.apply_segment(descriptor, *spool_receipt)?,
            InstantJournalCommand::BindMultipart { binding } => {
                self.require_live_upload_state()?;
                let binding = binding.validate()?;
                if binding.session_id != self.session_id
                    || self.segments.len() > binding.maximum_parts as usize
                    || self
                        .segments
                        .values()
                        .any(|segment| segment.descriptor.bytes > binding.maximum_part_bytes)
                {
                    return Err(InstantError::MultipartBindingConflict);
                }
                match self.multipart {
                    Some(existing) if existing == binding => {}
                    Some(_) => return Err(InstantError::MultipartBindingConflict),
                    None => self.multipart = Some(binding),
                }
            }
            InstantJournalCommand::RenewMultipart { binding } => {
                self.require_live_upload_state()?;
                let binding = binding.validate()?;
                let current = self.multipart.ok_or(InstantError::MultipartNotBound)?;
                if binding.session_id != self.session_id
                    || binding.session_id != current.session_id
                    || binding.upload_id != current.upload_id
                    || binding.generation <= current.generation
                    || binding.expires_at_ns <= current.expires_at_ns
                    || binding.minimum_part_bytes != current.minimum_part_bytes
                    || binding.maximum_part_bytes != current.maximum_part_bytes
                    || binding.maximum_parts != current.maximum_parts
                {
                    return Err(InstantError::InvalidMultipartRenewal);
                }
                self.multipart = Some(binding);
                for segment in self.segments.values_mut() {
                    if !matches!(segment.upload, SegmentUploadJournalState::Verified(_)) {
                        segment.upload = SegmentUploadJournalState::Queued {
                            attempt: upload_attempt(&segment.upload),
                            eligible_at_ns: now_ns,
                        };
                    }
                }
            }
            InstantJournalCommand::ClaimPart {
                index,
                worker_id,
                work_kind,
                attempt,
                lease_until_ns,
            } => {
                self.require_live_upload_state()?;
                let binding = self.multipart.ok_or(InstantError::MultipartNotBound)?;
                if binding.expires_at_ns <= now_ns || *lease_until_ns <= now_ns || *attempt == 0 {
                    return Err(InstantError::InvalidUploadClaim);
                }
                let segment = self
                    .segments
                    .get_mut(index)
                    .ok_or(InstantError::UnknownSegment(*index))?;
                let eligible = match segment.upload {
                    SegmentUploadJournalState::Queued {
                        attempt: prior,
                        eligible_at_ns,
                    }
                    | SegmentUploadJournalState::Deferred {
                        attempt: prior,
                        eligible_at_ns,
                        ..
                    } => {
                        *work_kind == InstantPartWorkKind::Upload
                            && eligible_at_ns <= now_ns
                            && *attempt == prior.saturating_add(1)
                    }
                    SegmentUploadJournalState::InFlight {
                        attempt: prior,
                        lease_until_ns: prior_lease,
                        work_kind: prior_work,
                        ..
                    } => {
                        *work_kind == prior_work
                            && prior_lease <= now_ns
                            && *attempt == prior.saturating_add(1)
                    }
                    SegmentUploadJournalState::ProbeRequired {
                        attempt: prior,
                        eligible_at_ns,
                    } => {
                        *work_kind == InstantPartWorkKind::Probe
                            && eligible_at_ns <= now_ns
                            && *attempt == prior.saturating_add(1)
                    }
                    SegmentUploadJournalState::Verified(_) => false,
                };
                if !eligible {
                    return Err(InstantError::PartNotClaimable(*index));
                }
                segment.upload = SegmentUploadJournalState::InFlight {
                    worker_id: *worker_id,
                    fence: self.fence,
                    work_kind: *work_kind,
                    attempt: *attempt,
                    lease_until_ns: *lease_until_ns,
                };
            }
            InstantJournalCommand::DeferPart {
                index,
                attempt,
                eligible_at_ns,
                reason,
            } => {
                self.require_live_upload_state()?;
                let segment = self.inflight_segment(*index, *attempt)?;
                if *eligible_at_ns < now_ns {
                    return Err(InstantError::InvalidRetrySchedule);
                }
                segment.upload = SegmentUploadJournalState::Deferred {
                    attempt: *attempt,
                    eligible_at_ns: *eligible_at_ns,
                    reason: *reason,
                };
            }
            InstantJournalCommand::RequirePartProbe {
                index,
                attempt,
                eligible_at_ns,
            } => {
                self.require_live_upload_state()?;
                let segment = self.inflight_segment(*index, *attempt)?;
                if *eligible_at_ns < now_ns {
                    return Err(InstantError::InvalidRetrySchedule);
                }
                segment.upload = SegmentUploadJournalState::ProbeRequired {
                    attempt: *attempt,
                    eligible_at_ns: *eligible_at_ns,
                };
            }
            InstantJournalCommand::VerifyPart { index, receipt } => {
                self.require_live_upload_state()?;
                let binding = self.multipart.ok_or(InstantError::MultipartNotBound)?;
                let segment = self
                    .segments
                    .get_mut(index)
                    .ok_or(InstantError::UnknownSegment(*index))?;
                let receipt = receipt.validate_for(binding, &segment.descriptor)?;
                match segment.upload {
                    SegmentUploadJournalState::Verified(existing) if existing == receipt => {}
                    SegmentUploadJournalState::Verified(_) => {
                        return Err(InstantError::PartReceiptConflict(*index));
                    }
                    _ => segment.upload = SegmentUploadJournalState::Verified(receipt),
                }
            }
            InstantJournalCommand::EvictSpool {
                index,
                proof,
                policy,
            } => {
                let segment = self
                    .segments
                    .get_mut(index)
                    .ok_or(InstantError::UnknownSegment(*index))?;
                let SegmentUploadJournalState::Verified(receipt) = segment.upload else {
                    return Err(InstantError::RemoteDurabilityUnproven);
                };
                if proof.part_receipt != receipt
                    || (*policy == SpoolEvictionPolicy::AfterFinalObject
                        && proof.final_object != self.multipart_complete)
                {
                    return Err(InstantError::RemoteDurabilityUnproven);
                }
                segment.spool_present = false;
            }
            InstantJournalCommand::RequestFinalize { job_generation } => {
                self.require_state(&[
                    InstantJournalState::CapturingOnline,
                    InstantJournalState::CapturingOffline,
                ])?;
                if self.multipart.is_none() {
                    return Err(InstantError::MultipartNotBound);
                }
                if *job_generation == 0 {
                    return Err(InstantError::InvalidJobGeneration);
                }
                if self.segments.values().any(|segment| {
                    !matches!(segment.upload, SegmentUploadJournalState::Verified(_))
                }) {
                    return Err(InstantError::SegmentsNotRemotelyVerified);
                }
                let binding = self.multipart.ok_or(InstantError::MultipartNotBound)?;
                if self
                    .segments
                    .values()
                    .take(self.segments.len().saturating_sub(1))
                    .any(|segment| segment.descriptor.bytes < binding.minimum_part_bytes)
                {
                    return Err(InstantError::MultipartPartSizeMismatch);
                }
                let manifest = InstantManifest::from_segments(
                    self.session_id,
                    self.segments
                        .values()
                        .map(|segment| segment.descriptor.clone()),
                )?;
                self.manifest = Some(manifest);
                self.job_generation = Some(*job_generation);
                self.state = InstantJournalState::Finalizing;
            }
            InstantJournalCommand::CompleteMultipart { receipt } => {
                self.require_state(&[InstantJournalState::Finalizing])?;
                self.validate_multipart_complete(*receipt)?;
                match self.multipart_complete {
                    Some(existing) if existing == *receipt => {}
                    Some(_) => return Err(InstantError::MultipartCompleteConflict),
                    None => self.multipart_complete = Some(*receipt),
                }
            }
            InstantJournalCommand::SealFinalizeRequest => {
                self.require_state(&[InstantJournalState::Finalizing])?;
                if self.finalize_request.is_none() {
                    let committed_revision = self
                        .revision
                        .checked_add(1)
                        .ok_or(InstantError::JournalCorrupt)?;
                    let manifest = self.manifest.clone().ok_or(InstantError::ManifestMissing)?;
                    let multipart = self
                        .multipart_complete
                        .ok_or(InstantError::MultipartNotComplete)?;
                    let job_generation = self
                        .job_generation
                        .ok_or(InstantError::InvalidJobGeneration)?;
                    let job_id = deterministic_job_id(self.session_id, manifest.digest)?;
                    self.finalize_request = Some(InstantFinalizeRequest::new(
                        self.session_id,
                        committed_revision,
                        self.fence,
                        manifest,
                        multipart,
                        job_id,
                        job_generation,
                    )?);
                }
            }
            InstantJournalCommand::Publish { receipt }
            | InstantJournalCommand::ApplyCallback { receipt } => {
                self.apply_publication(*receipt)?;
            }
            InstantJournalCommand::Cancel => {
                if self.state == InstantJournalState::Tombstoned {
                    return Ok(());
                }
                self.state = InstantJournalState::Tombstoned;
            }
            InstantJournalCommand::MarkRecoverableFailure => {
                if self.state.is_terminal() {
                    return Err(InstantError::InvalidState(self.state));
                }
                self.state = InstantJournalState::RecoverableFailure;
            }
        }
        Ok(())
    }

    fn apply_segment(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
        spool_receipt: SpoolCommitReceipt,
    ) -> Result<(), InstantError> {
        self.require_state(&[
            InstantJournalState::CapturingOnline,
            InstantJournalState::CapturingOffline,
        ])?;
        if descriptor.session_id != self.session_id
            || descriptor.index != self.segments.len() as u32
            || spool_receipt.segment_index != descriptor.index
            || spool_receipt.segment_identity != descriptor.identity
            || spool_receipt.bytes != descriptor.bytes
            || !spool_receipt.durable
        {
            return Err(InstantError::InvalidSpoolReceipt);
        }
        if let Some(binding) = self.multipart
            && (descriptor.bytes > binding.maximum_part_bytes
                || descriptor.index.saturating_add(1) > binding.maximum_parts)
        {
            return Err(InstantError::MultipartPartSizeMismatch);
        }
        let expected_start = self.segments.values().next_back().map_or(0, |segment| {
            segment
                .descriptor
                .start_ns
                .saturating_add(segment.descriptor.duration_ns)
        });
        if descriptor.start_ns != expected_start {
            return Err(InstantError::SegmentContinuity {
                expected_index: self.segments.len() as u32,
                found_index: descriptor.index,
            });
        }
        self.segments.insert(
            descriptor.index,
            InstantJournalSegment {
                descriptor: descriptor.clone(),
                spool_receipt,
                spool_present: true,
                upload: SegmentUploadJournalState::Queued {
                    attempt: 0,
                    eligible_at_ns: 0,
                },
            },
        );
        Ok(())
    }

    fn validate_multipart_complete(
        &self,
        receipt: InstantMultipartCompleteReceipt,
    ) -> Result<(), InstantError> {
        let binding = self.multipart.ok_or(InstantError::MultipartNotBound)?;
        let manifest = self
            .manifest
            .as_ref()
            .ok_or(InstantError::ManifestMissing)?;
        let ordered_parts_digest = self.ordered_parts_digest()?;
        if receipt.session_id != self.session_id
            || receipt.session_id != binding.session_id
            || receipt.upload_id != binding.upload_id
            || receipt.upload_generation != binding.generation
            || receipt.manifest_digest != manifest.digest
            || receipt.ordered_parts_digest != ordered_parts_digest
            || receipt.object.instant_manifest != manifest.digest
            || receipt.object.ordered_parts_digest != ordered_parts_digest
            || receipt.object.bytes != manifest.total_bytes
        {
            return Err(InstantError::InvalidMultipartCompleteReceipt);
        }
        Ok(())
    }

    fn apply_publication(&mut self, receipt: InstantFinalizeReceipt) -> Result<(), InstantError> {
        if self.state == InstantJournalState::Tombstoned {
            return Err(InstantError::TombstoneSealed);
        }
        if self.state == InstantJournalState::Ready {
            return if self.publication == Some(receipt) {
                Ok(())
            } else {
                Err(InstantError::PublishConflict)
            };
        }
        self.require_state(&[InstantJournalState::Finalizing])?;
        let expected = self.finalize_request()?;
        if receipt.request_digest != expected.digest
            || receipt.job_id != expected.job_id
            || receipt.job_generation != expected.job_generation
            || receipt.object != expected.multipart.object
            || receipt.playable_master.object_id != expected.multipart.object.object_id
            || receipt.playable_master.immutable_digest != expected.multipart.object.object_version
            || !receipt.playable_master.distribution_eligible
        {
            return Err(InstantError::InvalidFinalizeReceipt);
        }
        self.publication = Some(receipt);
        self.state = InstantJournalState::Ready;
        Ok(())
    }

    fn inflight_segment(
        &mut self,
        index: u32,
        attempt: u16,
    ) -> Result<&mut InstantJournalSegment, InstantError> {
        let segment = self
            .segments
            .get_mut(&index)
            .ok_or(InstantError::UnknownSegment(index))?;
        match segment.upload {
            SegmentUploadJournalState::InFlight {
                fence,
                attempt: active_attempt,
                ..
            } if fence == self.fence && active_attempt == attempt => Ok(segment),
            _ => Err(InstantError::StaleUploadClaim),
        }
    }

    fn require_live_upload_state(&self) -> Result<(), InstantError> {
        self.require_state(&[
            InstantJournalState::CapturingOnline,
            InstantJournalState::CapturingOffline,
            InstantJournalState::Finalizing,
        ])
    }

    fn require_state(&self, allowed: &[InstantJournalState]) -> Result<(), InstantError> {
        if allowed.contains(&self.state) {
            Ok(())
        } else {
            Err(InstantError::InvalidState(self.state))
        }
    }

    fn ordered_parts_digest(&self) -> Result<Sha256Digest, InstantError> {
        let mut bytes = Vec::with_capacity(64 + self.segments.len() * 32);
        bytes.extend_from_slice(b"frame.instant.ordered-parts.v1\0");
        for segment in self.segments.values() {
            let SegmentUploadJournalState::Verified(receipt) = segment.upload else {
                return Err(InstantError::SegmentsNotRemotelyVerified);
            };
            bytes.extend_from_slice(&receipt.digest().canonical_bytes());
        }
        Ok(strong_sha256(&bytes))
    }

    pub fn finalize_request(&self) -> Result<InstantFinalizeRequest, InstantError> {
        self.require_state(&[InstantJournalState::Finalizing])?;
        self.finalize_request
            .clone()
            .ok_or(InstantError::FinalizeRequestNotSealed)
    }

    fn outcome_digest(&self, kind: InstantOperationKind) -> Sha256Digest {
        let mut bytes = Vec::with_capacity(160);
        bytes.extend_from_slice(b"frame.instant.journal-outcome.v1\0");
        bytes.extend_from_slice(&self.session_id.canonical_bytes());
        bytes.extend_from_slice(&self.revision.to_be_bytes());
        bytes.extend_from_slice(&self.fence.get().to_be_bytes());
        bytes.push(journal_state_tag(self.state));
        bytes.push(operation_kind_tag(kind));
        bytes.extend_from_slice(&(self.segments.len() as u32).to_be_bytes());
        if let Some(manifest) = &self.manifest {
            bytes.extend_from_slice(&manifest.digest.canonical_bytes());
        }
        if let Some(publication) = self.publication {
            bytes.extend_from_slice(&publication.publication_id.canonical_bytes());
        }
        strong_sha256(&bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantJournalTransition {
    pub expected_revision: u64,
    pub expected_fence: InstantFence,
    pub snapshot: InstantJournalSnapshot,
    pub receipt: InstantOperationReceipt,
    pub requires_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantJournalCasRequest {
    pub session_id: InstantSessionId,
    pub expected_revision: u64,
    pub expected_fence: InstantFence,
    pub next: InstantJournalSnapshot,
    pub receipt: InstantOperationReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalPortOutcome<T> {
    Committed(T),
    Conflict(Box<InstantJournalSnapshot>),
    AcknowledgementLost,
}

pub trait InstantJournalPort: fmt::Debug {
    fn load(
        &mut self,
        session_id: InstantSessionId,
    ) -> Result<Option<InstantJournalSnapshot>, InstantError>;
    fn create(
        &mut self,
        initial: InstantJournalSnapshot,
    ) -> Result<JournalPortOutcome<InstantJournalSnapshot>, InstantError>;
    fn compare_and_swap(
        &mut self,
        request: InstantJournalCasRequest,
    ) -> Result<JournalPortOutcome<InstantJournalSnapshot>, InstantError>;
}

#[derive(Debug)]
pub struct DurableInstantJournal<J: InstantJournalPort> {
    port: J,
    snapshot: InstantJournalSnapshot,
}

impl<J: InstantJournalPort> DurableInstantJournal<J> {
    pub fn create(mut port: J, initial: InstantJournalSnapshot) -> Result<Self, InstantError> {
        initial.validate_integrity()?;
        let session_id = initial.session_id;
        match port.create(initial.clone())? {
            JournalPortOutcome::Committed(snapshot) => {
                validate_loaded_snapshot(session_id, &snapshot)?;
                Ok(Self { port, snapshot })
            }
            JournalPortOutcome::Conflict(snapshot) => {
                validate_loaded_snapshot(session_id, &snapshot)?;
                Ok(Self {
                    port,
                    snapshot: *snapshot,
                })
            }
            JournalPortOutcome::AcknowledgementLost => {
                let snapshot = port
                    .load(session_id)?
                    .ok_or(InstantError::AmbiguousJournalCommit)?;
                validate_loaded_snapshot(session_id, &snapshot)?;
                Ok(Self { port, snapshot })
            }
        }
    }

    pub fn recover(mut port: J, session_id: InstantSessionId) -> Result<Self, InstantError> {
        let snapshot = port.load(session_id)?.ok_or(InstantError::JournalMissing)?;
        validate_loaded_snapshot(session_id, &snapshot)?;
        Ok(Self { port, snapshot })
    }

    #[must_use]
    pub const fn snapshot(&self) -> &InstantJournalSnapshot {
        &self.snapshot
    }

    pub fn apply(
        &mut self,
        operation_id: InstantOperationId,
        command: InstantJournalCommand,
        now_ns: u64,
    ) -> Result<InstantOperationReceipt, InstantError> {
        let transition = self.snapshot.transition(
            self.snapshot.revision,
            self.snapshot.fence,
            operation_id,
            command,
            now_ns,
        )?;
        if !transition.requires_write {
            return Ok(transition.receipt);
        }
        let request = InstantJournalCasRequest {
            session_id: self.snapshot.session_id,
            expected_revision: transition.expected_revision,
            expected_fence: transition.expected_fence,
            next: transition.snapshot.clone(),
            receipt: transition.receipt,
        };
        match self.port.compare_and_swap(request)? {
            JournalPortOutcome::Committed(snapshot) => {
                validate_loaded_snapshot(self.snapshot.session_id, &snapshot)?;
                self.snapshot = snapshot;
                Ok(transition.receipt)
            }
            JournalPortOutcome::Conflict(snapshot) => {
                validate_loaded_snapshot(self.snapshot.session_id, &snapshot)?;
                self.snapshot = *snapshot;
                self.reconcile_operation(transition.receipt)
            }
            JournalPortOutcome::AcknowledgementLost => {
                self.snapshot = self
                    .port
                    .load(self.snapshot.session_id)?
                    .ok_or(InstantError::AmbiguousJournalCommit)?;
                validate_loaded_snapshot(self.snapshot.session_id, &self.snapshot)?;
                self.reconcile_operation(transition.receipt)
            }
        }
    }

    fn reconcile_operation(
        &self,
        expected: InstantOperationReceipt,
    ) -> Result<InstantOperationReceipt, InstantError> {
        match self.snapshot.operation_receipt(expected.operation_id) {
            Some(receipt) if receipt == expected => Ok(receipt),
            Some(_) => Err(InstantError::OperationKeyConflict),
            None => Err(InstantError::StaleJournal),
        }
    }

    pub fn into_port(self) -> J {
        self.port
    }
}

fn validate_loaded_snapshot(
    expected_session: InstantSessionId,
    snapshot: &InstantJournalSnapshot,
) -> Result<(), InstantError> {
    if snapshot.session_id != expected_session {
        return Err(InstantError::SessionBindingMismatch);
    }
    snapshot.validate_integrity()
}

#[derive(Clone, PartialEq, Eq)]
pub struct InstantFinalizeRequest {
    protocol_version: u16,
    session_id: InstantSessionId,
    journal_revision: u64,
    fence: InstantFence,
    manifest: InstantManifest,
    multipart: InstantMultipartCompleteReceipt,
    job_id: InstantJobId,
    job_generation: u64,
    digest: Sha256Digest,
}

impl fmt::Debug for InstantFinalizeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeRequest")
            .field("protocol_version", &self.protocol_version)
            .field("journal_revision", &self.journal_revision)
            .field("segment_count", &self.manifest.segment_count)
            .field("total_bytes", &self.manifest.total_bytes)
            .field("job_generation", &self.job_generation)
            .finish_non_exhaustive()
    }
}

impl InstantFinalizeRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: InstantSessionId,
        journal_revision: u64,
        fence: InstantFence,
        manifest: InstantManifest,
        multipart: InstantMultipartCompleteReceipt,
        job_id: InstantJobId,
        job_generation: u64,
    ) -> Result<Self, InstantError> {
        if journal_revision == 0
            || manifest.session_id != session_id
            || multipart.session_id != session_id
            || multipart.manifest_digest != manifest.digest
            || multipart.object.instant_manifest != manifest.digest
            || multipart.object.bytes != manifest.total_bytes
            || job_generation == 0
        {
            return Err(InstantError::InvalidFinalizeRequest);
        }
        let mut request = Self {
            protocol_version: INSTANT_FINALIZE_VERSION,
            session_id,
            journal_revision,
            fence,
            manifest,
            multipart,
            job_id,
            job_generation,
            digest: strong_sha256(b"pending-finalize-request"),
        };
        request.digest = request.compute_digest();
        Ok(request)
    }

    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    #[must_use]
    pub const fn job_id(&self) -> InstantJobId {
        self.job_id
    }

    #[must_use]
    pub const fn job_generation(&self) -> u64 {
        self.job_generation
    }

    #[must_use]
    pub const fn multipart(&self) -> InstantMultipartCompleteReceipt {
        self.multipart
    }

    fn compute_digest(&self) -> Sha256Digest {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(b"frame.instant.finalize-request.v1\0");
        bytes.extend_from_slice(&self.protocol_version.to_be_bytes());
        bytes.extend_from_slice(&self.session_id.canonical_bytes());
        bytes.extend_from_slice(&self.journal_revision.to_be_bytes());
        bytes.extend_from_slice(&self.fence.get().to_be_bytes());
        bytes.extend_from_slice(&self.manifest.digest.canonical_bytes());
        append_complete_receipt(&mut bytes, self.multipart);
        bytes.extend_from_slice(&self.job_id.canonical_bytes());
        bytes.extend_from_slice(&self.job_generation.to_be_bytes());
        strong_sha256(&bytes)
    }
}

impl InstantJournalSnapshot {
    fn validate_integrity(&self) -> Result<(), InstantError> {
        if self.protocol_version != INSTANT_PROTOCOL_VERSION
            || self.journal_version != INSTANT_JOURNAL_VERSION
            || self.revision == 0
            || self.segments.len() > MAX_INSTANT_SEGMENTS as usize
            || self.receipts.len() > MAX_INSTANT_OPERATION_RECEIPTS
            || self.revision != self.receipts.len() as u64 + 1
            || self.manifest.is_some() != self.job_generation.is_some()
            || (self.manifest.is_some() && self.multipart.is_none())
        {
            return Err(InstantError::JournalCorrupt);
        }
        if self
            .multipart
            .is_some_and(|binding| binding.session_id != self.session_id)
        {
            return Err(InstantError::JournalCorrupt);
        }
        for (expected, (index, segment)) in self.segments.iter().enumerate() {
            let expected = u32::try_from(expected).map_err(|_| InstantError::JournalCorrupt)?;
            if *index != expected
                || segment.descriptor.index != expected
                || segment.descriptor.session_id != self.session_id
                || segment.descriptor.compute_identity() != segment.descriptor.identity
                || segment.spool_receipt.segment_index != expected
                || segment.spool_receipt.segment_identity != segment.descriptor.identity
                || segment.spool_receipt.bytes != segment.descriptor.bytes
                || !segment.spool_receipt.durable
            {
                return Err(InstantError::JournalCorrupt);
            }
            if let Some(binding) = self.multipart {
                if segment.descriptor.bytes > binding.maximum_part_bytes
                    || expected.saturating_add(1) > binding.maximum_parts
                {
                    return Err(InstantError::JournalCorrupt);
                }
                if let SegmentUploadJournalState::Verified(receipt) = segment.upload {
                    receipt.validate_persisted_for(binding, &segment.descriptor)?;
                }
            } else if matches!(segment.upload, SegmentUploadJournalState::Verified(_)) {
                return Err(InstantError::JournalCorrupt);
            }
        }
        if let Some(manifest) = &self.manifest {
            let rebuilt = InstantManifest::from_segments(
                self.session_id,
                self.segments
                    .values()
                    .map(|segment| segment.descriptor.clone()),
            )?;
            if *manifest != rebuilt {
                return Err(InstantError::JournalCorrupt);
            }
        }
        if let Some(complete) = self.multipart_complete {
            self.validate_multipart_complete(complete)?;
        }
        if let Some(request) = &self.finalize_request {
            if request.journal_revision > self.revision
                || request.manifest
                    != *self.manifest.as_ref().ok_or(InstantError::JournalCorrupt)?
                || Some(request.multipart) != self.multipart_complete
                || Some(request.job_generation) != self.job_generation
            {
                return Err(InstantError::JournalCorrupt);
            }
            let rebuilt = InstantFinalizeRequest::new(
                request.session_id,
                request.journal_revision,
                request.fence,
                request.manifest.clone(),
                request.multipart,
                request.job_id,
                request.job_generation,
            )?;
            if *request != rebuilt {
                return Err(InstantError::JournalCorrupt);
            }
            if !self.receipts.values().any(|receipt| {
                receipt.committed_revision == request.journal_revision
                    && receipt.kind == InstantOperationKind::FinalizeDispatch
            }) {
                return Err(InstantError::JournalCorrupt);
            }
        }
        if let Some(publication) = self.publication {
            let request = self
                .finalize_request
                .as_ref()
                .ok_or(InstantError::JournalCorrupt)?;
            validate_finalize_receipt(request, publication)?;
        }
        let mut committed_revisions = BTreeSet::new();
        for (operation_id, receipt) in &self.receipts {
            if receipt.operation_id != *operation_id
                || receipt.committed_revision < 2
                || receipt.committed_revision > self.revision
                || !committed_revisions.insert(receipt.committed_revision)
            {
                return Err(InstantError::JournalCorrupt);
            }
        }
        if committed_revisions.iter().copied().ne(2..=self.revision) {
            return Err(InstantError::JournalCorrupt);
        }
        let all_segments_verified = self
            .segments
            .values()
            .all(|segment| matches!(segment.upload, SegmentUploadJournalState::Verified(_)));
        let invalid_state = match self.state {
            InstantJournalState::Created => {
                !self.segments.is_empty()
                    || self.multipart.is_some()
                    || self.manifest.is_some()
                    || self.multipart_complete.is_some()
                    || self.job_generation.is_some()
                    || self.finalize_request.is_some()
                    || self.publication.is_some()
            }
            InstantJournalState::CapturingOnline | InstantJournalState::CapturingOffline => {
                self.manifest.is_some()
                    || self.multipart_complete.is_some()
                    || self.job_generation.is_some()
                    || self.finalize_request.is_some()
                    || self.publication.is_some()
            }
            InstantJournalState::Finalizing => {
                self.multipart.is_none()
                    || self.manifest.is_none()
                    || self.job_generation.is_none()
                    || !all_segments_verified
                    || self.publication.is_some()
                    || (self.finalize_request.is_some() && self.multipart_complete.is_none())
            }
            InstantJournalState::Ready => {
                self.multipart.is_none()
                    || self.manifest.is_none()
                    || self.job_generation.is_none()
                    || !all_segments_verified
                    || self.multipart_complete.is_none()
                    || self.finalize_request.is_none()
                    || self.publication.is_none()
            }
            InstantJournalState::Tombstoned | InstantJournalState::RecoverableFailure => false,
        };
        if invalid_state {
            return Err(InstantError::JournalCorrupt);
        }
        Ok(())
    }
}

/// Canonical bounded binary persistence for adapters that cannot retain Rust
/// values across process restart. The encoded bytes are sensitive state and
/// must not be logged; decoding re-runs every core invariant.
pub struct InstantJournalCodec;

impl InstantJournalCodec {
    pub fn encode(snapshot: &InstantJournalSnapshot) -> Result<Vec<u8>, InstantError> {
        snapshot.validate_integrity()?;
        let mut writer = JournalWriter::default();
        writer.raw(b"FRINJNL1");
        writer.u16(INSTANT_JOURNAL_VERSION);
        writer.u16(snapshot.protocol_version);
        writer.raw(&snapshot.session_id.canonical_bytes());
        writer.u64(snapshot.revision);
        writer.u64(snapshot.fence.get());
        writer.u8(journal_state_tag(snapshot.state));
        writer
            .u32(u32::try_from(snapshot.segments.len()).map_err(|_| InstantError::JournalCorrupt)?);
        for segment in snapshot.segments.values() {
            encode_segment_descriptor(&mut writer, &segment.descriptor)?;
            encode_spool_commit_receipt(&mut writer, segment.spool_receipt);
            writer.boolean(segment.spool_present);
            encode_upload_state(&mut writer, &segment.upload);
        }
        encode_optional_binding(&mut writer, snapshot.multipart);
        encode_optional_manifest(&mut writer, snapshot.manifest.as_ref())?;
        encode_optional_complete(&mut writer, snapshot.multipart_complete);
        encode_optional_u64(&mut writer, snapshot.job_generation);
        encode_optional_finalize_request(&mut writer, snapshot.finalize_request.as_ref());
        encode_optional_publication(&mut writer, snapshot.publication);
        writer
            .u32(u32::try_from(snapshot.receipts.len()).map_err(|_| InstantError::JournalCorrupt)?);
        for receipt in snapshot.receipts.values() {
            writer.raw(&receipt.operation_id.canonical_bytes());
            writer.u8(operation_kind_tag(receipt.kind));
            writer.u64(receipt.committed_revision);
            writer.digest(receipt.command_digest);
            writer.digest(receipt.outcome_digest);
        }
        let integrity = strong_sha256(&writer.bytes);
        writer.digest(integrity);
        if writer.bytes.len() > MAX_INSTANT_JOURNAL_BYTES {
            return Err(InstantError::JournalEncodingTooLarge);
        }
        Ok(writer.bytes)
    }

    pub fn decode(encoded: &[u8]) -> Result<InstantJournalSnapshot, InstantError> {
        if encoded.len() > MAX_INSTANT_JOURNAL_BYTES {
            return Err(InstantError::JournalEncodingTooLarge);
        }
        let payload_length = encoded
            .len()
            .checked_sub(32)
            .ok_or(InstantError::MalformedJournalEncoding)?;
        let (payload, encoded_integrity) = encoded.split_at(payload_length);
        let encoded_integrity = Sha256Digest::from_bytes(
            encoded_integrity
                .try_into()
                .map_err(|_| InstantError::MalformedJournalEncoding)?,
        )
        .map_err(|_| InstantError::MalformedJournalEncoding)?;
        if strong_sha256(payload) != encoded_integrity {
            return Err(InstantError::MalformedJournalEncoding);
        }
        let mut reader = JournalReader::new(payload);
        if reader.array::<8>()? != *b"FRINJNL1" {
            return Err(InstantError::MalformedJournalEncoding);
        }
        let journal_version = reader.u16()?;
        let protocol_version = reader.u16()?;
        if journal_version != INSTANT_JOURNAL_VERSION
            || protocol_version != INSTANT_PROTOCOL_VERSION
        {
            return Err(InstantError::UnsupportedJournalVersion);
        }
        let session_id = decode_session_id(&mut reader)?;
        let revision = reader.u64()?;
        let fence =
            InstantFence::new(reader.u64()?).map_err(|_| InstantError::MalformedJournalEncoding)?;
        let state = decode_journal_state(reader.u8()?)?;
        let segment_count = reader.u32()?;
        if segment_count > MAX_INSTANT_SEGMENTS {
            return Err(InstantError::MalformedJournalEncoding);
        }
        let mut segments = BTreeMap::new();
        for _ in 0..segment_count {
            let descriptor = decode_segment_descriptor(&mut reader)?;
            let spool_receipt = decode_spool_commit_receipt(&mut reader)?;
            let spool_present = reader.boolean()?;
            let upload = decode_upload_state(&mut reader)?;
            let index = descriptor.index;
            if segments
                .insert(
                    index,
                    InstantJournalSegment {
                        descriptor,
                        spool_receipt,
                        spool_present,
                        upload,
                    },
                )
                .is_some()
            {
                return Err(InstantError::MalformedJournalEncoding);
            }
        }
        let multipart = decode_optional_binding(&mut reader)?;
        let manifest = decode_optional_manifest(&mut reader, session_id, &segments)?;
        let multipart_complete = decode_optional_complete(&mut reader)?;
        let job_generation = decode_optional_u64(&mut reader)?;
        let finalize_request =
            decode_optional_finalize_request(&mut reader, manifest.as_ref(), multipart_complete)?;
        let publication = decode_optional_publication(&mut reader)?;
        let receipt_count = reader.u32()? as usize;
        if receipt_count > MAX_INSTANT_OPERATION_RECEIPTS {
            return Err(InstantError::MalformedJournalEncoding);
        }
        let mut receipts = BTreeMap::new();
        for _ in 0..receipt_count {
            let operation_id = decode_operation_id(&mut reader)?;
            let receipt = InstantOperationReceipt {
                operation_id,
                kind: decode_operation_kind(reader.u8()?)?,
                committed_revision: reader.u64()?,
                command_digest: reader.digest()?,
                outcome_digest: reader.digest()?,
            };
            if receipts.insert(operation_id, receipt).is_some() {
                return Err(InstantError::MalformedJournalEncoding);
            }
        }
        reader.finish()?;
        let snapshot = InstantJournalSnapshot {
            protocol_version,
            journal_version,
            session_id,
            revision,
            fence,
            state,
            segments,
            multipart,
            manifest,
            multipart_complete,
            job_generation,
            finalize_request,
            publication,
            receipts,
        };
        snapshot.validate_integrity()?;
        Ok(snapshot)
    }
}

#[derive(Default)]
struct JournalWriter {
    bytes: Vec<u8>,
}

impl JournalWriter {
    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn boolean(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u16(&mut self, value: u16) {
        self.raw(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.raw(&value.to_be_bytes());
    }

    fn digest(&mut self, value: Sha256Digest) {
        self.raw(&value.canonical_bytes());
    }
}

struct JournalReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> JournalReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], InstantError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(InstantError::MalformedJournalEncoding)?;
        let source = self
            .bytes
            .get(self.cursor..end)
            .ok_or(InstantError::MalformedJournalEncoding)?;
        let mut value = [0_u8; N];
        value.copy_from_slice(source);
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, InstantError> {
        Ok(self.array::<1>()?[0])
    }

    fn boolean(&mut self) -> Result<bool, InstantError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(InstantError::MalformedJournalEncoding),
        }
    }

    fn u16(&mut self) -> Result<u16, InstantError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, InstantError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, InstantError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn digest(&mut self) -> Result<Sha256Digest, InstantError> {
        Sha256Digest::from_bytes(self.array()?).map_err(|_| InstantError::MalformedJournalEncoding)
    }

    fn finish(self) -> Result<(), InstantError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(InstantError::MalformedJournalEncoding)
        }
    }
}

fn encode_segment_descriptor(
    writer: &mut JournalWriter,
    descriptor: &InstantSegmentDescriptor,
) -> Result<(), InstantError> {
    writer.u16(descriptor.protocol_version);
    writer.raw(&descriptor.session_id.canonical_bytes());
    writer.u32(descriptor.index);
    writer.u64(descriptor.start_ns);
    writer.u64(descriptor.duration_ns);
    writer.boolean(descriptor.starts_with_video_keyframe);
    writer.u8(match descriptor.container {
        InstantContainer::FragmentedMp4Cmaf => 1,
    });
    writer.u8(u8::try_from(descriptor.tracks.len()).map_err(|_| InstantError::JournalCorrupt)?);
    for track in &descriptor.tracks {
        writer.u16(track.track_number);
        writer.u8(track_role_tag(track.role));
        writer.u8(codec_tag(track.codec));
        writer.u32(track.timescale);
        writer.u32(track.sample_count);
        writer.u64(track.first_presentation_ns);
        writer.u64(track.duration_ns);
    }
    writer.u64(descriptor.bytes);
    writer.digest(descriptor.sha256);
    writer.digest(descriptor.identity);
    Ok(())
}

fn encode_spool_commit_receipt(writer: &mut JournalWriter, receipt: SpoolCommitReceipt) {
    writer.u32(receipt.segment_index);
    writer.digest(receipt.segment_identity);
    writer.u64(receipt.bytes);
    writer.digest(receipt.ciphertext_integrity);
    writer.boolean(receipt.durable);
}

fn encode_part_receipt(writer: &mut JournalWriter, receipt: InstantPartReceipt) {
    writer.raw(&receipt.session_id.canonical_bytes());
    writer.raw(&receipt.upload_id.canonical_bytes());
    writer.u64(receipt.upload_generation);
    writer.u32(receipt.part_number);
    writer.u64(receipt.bytes);
    writer.digest(receipt.sha256);
    writer.digest(receipt.segment_identity);
    writer.digest(receipt.provider_receipt_digest);
}

fn encode_upload_state(writer: &mut JournalWriter, state: &SegmentUploadJournalState) {
    match state {
        SegmentUploadJournalState::Queued {
            attempt,
            eligible_at_ns,
        } => {
            writer.u8(1);
            writer.u16(*attempt);
            writer.u64(*eligible_at_ns);
        }
        SegmentUploadJournalState::InFlight {
            worker_id,
            fence,
            work_kind,
            attempt,
            lease_until_ns,
        } => {
            writer.u8(2);
            writer.raw(&worker_id.canonical_bytes());
            writer.u64(fence.get());
            writer.u8(match work_kind {
                InstantPartWorkKind::Upload => 1,
                InstantPartWorkKind::Probe => 2,
            });
            writer.u16(*attempt);
            writer.u64(*lease_until_ns);
        }
        SegmentUploadJournalState::Deferred {
            attempt,
            eligible_at_ns,
            reason,
        } => {
            writer.u8(3);
            writer.u16(*attempt);
            writer.u64(*eligible_at_ns);
            writer.u8(defer_reason_tag(*reason));
        }
        SegmentUploadJournalState::ProbeRequired {
            attempt,
            eligible_at_ns,
        } => {
            writer.u8(4);
            writer.u16(*attempt);
            writer.u64(*eligible_at_ns);
        }
        SegmentUploadJournalState::Verified(receipt) => {
            writer.u8(5);
            encode_part_receipt(writer, *receipt);
        }
    }
}

fn encode_binding(writer: &mut JournalWriter, binding: InstantMultipartBinding) {
    writer.raw(&binding.session_id.canonical_bytes());
    writer.raw(&binding.upload_id.canonical_bytes());
    writer.u64(binding.expires_at_ns);
    writer.u64(binding.generation);
    writer.u64(binding.minimum_part_bytes);
    writer.u64(binding.maximum_part_bytes);
    writer.u32(binding.maximum_parts);
}

fn encode_optional_binding(writer: &mut JournalWriter, binding: Option<InstantMultipartBinding>) {
    writer.boolean(binding.is_some());
    if let Some(binding) = binding {
        encode_binding(writer, binding);
    }
}

fn encode_manifest(
    writer: &mut JournalWriter,
    manifest: &InstantManifest,
) -> Result<(), InstantError> {
    writer.u16(manifest.protocol_version);
    writer.u16(manifest.manifest_version);
    writer.raw(&manifest.session_id.canonical_bytes());
    writer.u32(manifest.segment_count);
    writer.u64(manifest.total_bytes);
    writer.u64(manifest.duration_ns);
    writer.u32(
        u32::try_from(manifest.segment_identities.len())
            .map_err(|_| InstantError::JournalCorrupt)?,
    );
    for identity in &manifest.segment_identities {
        writer.digest(*identity);
    }
    writer.digest(manifest.digest);
    Ok(())
}

fn encode_optional_manifest(
    writer: &mut JournalWriter,
    manifest: Option<&InstantManifest>,
) -> Result<(), InstantError> {
    writer.boolean(manifest.is_some());
    if let Some(manifest) = manifest {
        encode_manifest(writer, manifest)?;
    }
    Ok(())
}

fn encode_object_manifest(writer: &mut JournalWriter, object: InstantObjectManifest) {
    writer.raw(&object.object_id.canonical_bytes());
    writer.digest(object.object_version);
    writer.digest(object.instant_manifest);
    writer.digest(object.ordered_parts_digest);
    writer.u64(object.bytes);
}

fn encode_complete(writer: &mut JournalWriter, receipt: InstantMultipartCompleteReceipt) {
    writer.raw(&receipt.session_id.canonical_bytes());
    writer.raw(&receipt.upload_id.canonical_bytes());
    writer.u64(receipt.upload_generation);
    writer.digest(receipt.manifest_digest);
    writer.digest(receipt.ordered_parts_digest);
    encode_object_manifest(writer, receipt.object);
}

fn encode_optional_complete(
    writer: &mut JournalWriter,
    receipt: Option<InstantMultipartCompleteReceipt>,
) {
    writer.boolean(receipt.is_some());
    if let Some(receipt) = receipt {
        encode_complete(writer, receipt);
    }
}

fn encode_optional_u64(writer: &mut JournalWriter, value: Option<u64>) {
    writer.boolean(value.is_some());
    if let Some(value) = value {
        writer.u64(value);
    }
}

fn encode_finalize_request(writer: &mut JournalWriter, request: &InstantFinalizeRequest) {
    writer.u16(request.protocol_version);
    writer.raw(&request.session_id.canonical_bytes());
    writer.u64(request.journal_revision);
    writer.u64(request.fence.get());
    writer.digest(request.manifest.digest);
    encode_complete(writer, request.multipart);
    writer.raw(&request.job_id.canonical_bytes());
    writer.u64(request.job_generation);
    writer.digest(request.digest);
}

fn encode_optional_finalize_request(
    writer: &mut JournalWriter,
    request: Option<&InstantFinalizeRequest>,
) {
    writer.boolean(request.is_some());
    if let Some(request) = request {
        encode_finalize_request(writer, request);
    }
}

fn encode_publication(writer: &mut JournalWriter, receipt: InstantFinalizeReceipt) {
    writer.digest(receipt.request_digest);
    writer.raw(&receipt.publication_id.canonical_bytes());
    writer.raw(&receipt.job_id.canonical_bytes());
    writer.u64(receipt.job_generation);
    encode_object_manifest(writer, receipt.object);
    writer.raw(&receipt.playable_master.object_id.canonical_bytes());
    writer.digest(receipt.playable_master.immutable_digest);
    writer.boolean(receipt.playable_master.distribution_eligible);
}

fn encode_optional_publication(
    writer: &mut JournalWriter,
    receipt: Option<InstantFinalizeReceipt>,
) {
    writer.boolean(receipt.is_some());
    if let Some(receipt) = receipt {
        encode_publication(writer, receipt);
    }
}

fn decode_session_id(reader: &mut JournalReader<'_>) -> Result<InstantSessionId, InstantError> {
    InstantSessionId::from_csprng(reader.array()?)
        .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_operation_id(reader: &mut JournalReader<'_>) -> Result<InstantOperationId, InstantError> {
    InstantOperationId::from_csprng(reader.array()?)
        .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_worker_id(reader: &mut JournalReader<'_>) -> Result<InstantWorkerId, InstantError> {
    InstantWorkerId::from_csprng(reader.array()?)
        .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_upload_id(reader: &mut JournalReader<'_>) -> Result<InstantUploadId, InstantError> {
    InstantUploadId::from_csprng(reader.array()?)
        .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_object_id(reader: &mut JournalReader<'_>) -> Result<InstantObjectId, InstantError> {
    InstantObjectId::from_csprng(reader.array()?)
        .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_job_id(reader: &mut JournalReader<'_>) -> Result<InstantJobId, InstantError> {
    InstantJobId::from_csprng(reader.array()?).map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_publication_id(
    reader: &mut JournalReader<'_>,
) -> Result<InstantPublicationId, InstantError> {
    InstantPublicationId::from_csprng(reader.array()?)
        .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_track_role(tag: u8) -> Result<InstantTrackRole, InstantError> {
    match tag {
        1 => Ok(InstantTrackRole::ScreenVideo),
        2 => Ok(InstantTrackRole::CameraVideo),
        3 => Ok(InstantTrackRole::MixedAudio),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

fn decode_codec(tag: u8) -> Result<InstantCodec, InstantError> {
    match tag {
        1 => Ok(InstantCodec::H264Avc),
        2 => Ok(InstantCodec::AacLowComplexity),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

fn decode_segment_descriptor(
    reader: &mut JournalReader<'_>,
) -> Result<InstantSegmentDescriptor, InstantError> {
    let protocol_version = reader.u16()?;
    if protocol_version != INSTANT_PROTOCOL_VERSION {
        return Err(InstantError::UnsupportedJournalVersion);
    }
    let session_id = decode_session_id(reader)?;
    let index = reader.u32()?;
    let start_ns = reader.u64()?;
    let duration_ns = reader.u64()?;
    let keyframe = reader.boolean()?;
    let container = match reader.u8()? {
        1 => InstantContainer::FragmentedMp4Cmaf,
        _ => return Err(InstantError::MalformedJournalEncoding),
    };
    let track_count = usize::from(reader.u8()?);
    if track_count == 0 || track_count > MAX_INSTANT_TRACKS {
        return Err(InstantError::MalformedJournalEncoding);
    }
    let mut tracks = Vec::with_capacity(track_count);
    for _ in 0..track_count {
        tracks.push(
            InstantTrackMetadata::new(
                reader.u16()?,
                decode_track_role(reader.u8()?)?,
                decode_codec(reader.u8()?)?,
                reader.u32()?,
                reader.u32()?,
                reader.u64()?,
                reader.u64()?,
            )
            .map_err(|_| InstantError::MalformedJournalEncoding)?,
        );
    }
    let bytes = reader.u64()?;
    let sha256 = reader.digest()?;
    let stored_identity = reader.digest()?;
    let descriptor = InstantSegmentDescriptor::new(
        session_id,
        index,
        start_ns,
        duration_ns,
        keyframe,
        container,
        tracks,
        bytes,
        sha256,
    )
    .map_err(|_| InstantError::MalformedJournalEncoding)?;
    if descriptor.identity != stored_identity {
        return Err(InstantError::MalformedJournalEncoding);
    }
    Ok(descriptor)
}

fn decode_spool_commit_receipt(
    reader: &mut JournalReader<'_>,
) -> Result<SpoolCommitReceipt, InstantError> {
    Ok(SpoolCommitReceipt {
        segment_index: reader.u32()?,
        segment_identity: reader.digest()?,
        bytes: reader.u64()?,
        ciphertext_integrity: reader.digest()?,
        durable: reader.boolean()?,
    })
}

fn decode_part_receipt(reader: &mut JournalReader<'_>) -> Result<InstantPartReceipt, InstantError> {
    Ok(InstantPartReceipt {
        session_id: decode_session_id(reader)?,
        upload_id: decode_upload_id(reader)?,
        upload_generation: reader.u64()?,
        part_number: reader.u32()?,
        bytes: reader.u64()?,
        sha256: reader.digest()?,
        segment_identity: reader.digest()?,
        provider_receipt_digest: reader.digest()?,
    })
}

fn decode_work_kind(tag: u8) -> Result<InstantPartWorkKind, InstantError> {
    match tag {
        1 => Ok(InstantPartWorkKind::Upload),
        2 => Ok(InstantPartWorkKind::Probe),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

fn decode_defer_reason(tag: u8) -> Result<UploadDeferReason, InstantError> {
    match tag {
        1 => Ok(UploadDeferReason::Offline),
        2 => Ok(UploadDeferReason::Throttled),
        3 => Ok(UploadDeferReason::ProviderUnavailable),
        4 => Ok(UploadDeferReason::UploadExpired),
        5 => Ok(UploadDeferReason::LostAcknowledgement),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

fn decode_upload_state(
    reader: &mut JournalReader<'_>,
) -> Result<SegmentUploadJournalState, InstantError> {
    match reader.u8()? {
        1 => Ok(SegmentUploadJournalState::Queued {
            attempt: reader.u16()?,
            eligible_at_ns: reader.u64()?,
        }),
        2 => Ok(SegmentUploadJournalState::InFlight {
            worker_id: decode_worker_id(reader)?,
            fence: InstantFence::new(reader.u64()?)
                .map_err(|_| InstantError::MalformedJournalEncoding)?,
            work_kind: decode_work_kind(reader.u8()?)?,
            attempt: reader.u16()?,
            lease_until_ns: reader.u64()?,
        }),
        3 => Ok(SegmentUploadJournalState::Deferred {
            attempt: reader.u16()?,
            eligible_at_ns: reader.u64()?,
            reason: decode_defer_reason(reader.u8()?)?,
        }),
        4 => Ok(SegmentUploadJournalState::ProbeRequired {
            attempt: reader.u16()?,
            eligible_at_ns: reader.u64()?,
        }),
        5 => Ok(SegmentUploadJournalState::Verified(decode_part_receipt(
            reader,
        )?)),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

fn decode_binding(reader: &mut JournalReader<'_>) -> Result<InstantMultipartBinding, InstantError> {
    InstantMultipartBinding {
        session_id: decode_session_id(reader)?,
        upload_id: decode_upload_id(reader)?,
        expires_at_ns: reader.u64()?,
        generation: reader.u64()?,
        minimum_part_bytes: reader.u64()?,
        maximum_part_bytes: reader.u64()?,
        maximum_parts: reader.u32()?,
    }
    .validate()
    .map_err(|_| InstantError::MalformedJournalEncoding)
}

fn decode_optional_binding(
    reader: &mut JournalReader<'_>,
) -> Result<Option<InstantMultipartBinding>, InstantError> {
    if reader.boolean()? {
        Ok(Some(decode_binding(reader)?))
    } else {
        Ok(None)
    }
}

fn decode_optional_manifest(
    reader: &mut JournalReader<'_>,
    session_id: InstantSessionId,
    segments: &BTreeMap<u32, InstantJournalSegment>,
) -> Result<Option<InstantManifest>, InstantError> {
    if !reader.boolean()? {
        return Ok(None);
    }
    let protocol_version = reader.u16()?;
    let manifest_version = reader.u16()?;
    let encoded_session = decode_session_id(reader)?;
    let segment_count = reader.u32()?;
    let total_bytes = reader.u64()?;
    let duration_ns = reader.u64()?;
    let identity_count = reader.u32()?;
    if identity_count > MAX_INSTANT_SEGMENTS {
        return Err(InstantError::MalformedJournalEncoding);
    }
    let mut identities = Vec::with_capacity(identity_count as usize);
    for _ in 0..identity_count {
        identities.push(reader.digest()?);
    }
    let digest = reader.digest()?;
    let rebuilt = InstantManifest::from_segments(
        session_id,
        segments.values().map(|segment| segment.descriptor.clone()),
    )
    .map_err(|_| InstantError::MalformedJournalEncoding)?;
    if protocol_version != INSTANT_PROTOCOL_VERSION
        || manifest_version != INSTANT_MANIFEST_VERSION
        || encoded_session != session_id
        || segment_count != rebuilt.segment_count
        || total_bytes != rebuilt.total_bytes
        || duration_ns != rebuilt.duration_ns
        || identities != rebuilt.segment_identities
        || digest != rebuilt.digest
    {
        return Err(InstantError::MalformedJournalEncoding);
    }
    Ok(Some(rebuilt))
}

fn decode_object_manifest(
    reader: &mut JournalReader<'_>,
) -> Result<InstantObjectManifest, InstantError> {
    Ok(InstantObjectManifest {
        object_id: decode_object_id(reader)?,
        object_version: reader.digest()?,
        instant_manifest: reader.digest()?,
        ordered_parts_digest: reader.digest()?,
        bytes: reader.u64()?,
    })
}

fn decode_complete(
    reader: &mut JournalReader<'_>,
) -> Result<InstantMultipartCompleteReceipt, InstantError> {
    Ok(InstantMultipartCompleteReceipt {
        session_id: decode_session_id(reader)?,
        upload_id: decode_upload_id(reader)?,
        upload_generation: reader.u64()?,
        manifest_digest: reader.digest()?,
        ordered_parts_digest: reader.digest()?,
        object: decode_object_manifest(reader)?,
    })
}

fn decode_optional_complete(
    reader: &mut JournalReader<'_>,
) -> Result<Option<InstantMultipartCompleteReceipt>, InstantError> {
    if reader.boolean()? {
        Ok(Some(decode_complete(reader)?))
    } else {
        Ok(None)
    }
}

fn decode_optional_u64(reader: &mut JournalReader<'_>) -> Result<Option<u64>, InstantError> {
    if reader.boolean()? {
        Ok(Some(reader.u64()?))
    } else {
        Ok(None)
    }
}

fn decode_optional_finalize_request(
    reader: &mut JournalReader<'_>,
    manifest: Option<&InstantManifest>,
    complete: Option<InstantMultipartCompleteReceipt>,
) -> Result<Option<InstantFinalizeRequest>, InstantError> {
    if !reader.boolean()? {
        return Ok(None);
    }
    let protocol_version = reader.u16()?;
    let session_id = decode_session_id(reader)?;
    let journal_revision = reader.u64()?;
    let fence =
        InstantFence::new(reader.u64()?).map_err(|_| InstantError::MalformedJournalEncoding)?;
    let manifest_digest = reader.digest()?;
    let encoded_complete = decode_complete(reader)?;
    let job_id = decode_job_id(reader)?;
    let job_generation = reader.u64()?;
    let digest = reader.digest()?;
    let manifest = manifest
        .cloned()
        .ok_or(InstantError::MalformedJournalEncoding)?;
    if protocol_version != INSTANT_FINALIZE_VERSION
        || manifest_digest != manifest.digest
        || Some(encoded_complete) != complete
    {
        return Err(InstantError::MalformedJournalEncoding);
    }
    let request = InstantFinalizeRequest::new(
        session_id,
        journal_revision,
        fence,
        manifest,
        encoded_complete,
        job_id,
        job_generation,
    )
    .map_err(|_| InstantError::MalformedJournalEncoding)?;
    if request.digest != digest {
        return Err(InstantError::MalformedJournalEncoding);
    }
    Ok(Some(request))
}

fn decode_publication(
    reader: &mut JournalReader<'_>,
) -> Result<InstantFinalizeReceipt, InstantError> {
    Ok(InstantFinalizeReceipt {
        request_digest: reader.digest()?,
        publication_id: decode_publication_id(reader)?,
        job_id: decode_job_id(reader)?,
        job_generation: reader.u64()?,
        object: decode_object_manifest(reader)?,
        playable_master: PlayableMasterIdentity {
            object_id: decode_object_id(reader)?,
            immutable_digest: reader.digest()?,
            distribution_eligible: reader.boolean()?,
        },
    })
}

fn decode_optional_publication(
    reader: &mut JournalReader<'_>,
) -> Result<Option<InstantFinalizeReceipt>, InstantError> {
    if reader.boolean()? {
        Ok(Some(decode_publication(reader)?))
    } else {
        Ok(None)
    }
}

fn decode_journal_state(tag: u8) -> Result<InstantJournalState, InstantError> {
    match tag {
        1 => Ok(InstantJournalState::Created),
        2 => Ok(InstantJournalState::CapturingOnline),
        3 => Ok(InstantJournalState::CapturingOffline),
        4 => Ok(InstantJournalState::Finalizing),
        5 => Ok(InstantJournalState::Ready),
        6 => Ok(InstantJournalState::Tombstoned),
        7 => Ok(InstantJournalState::RecoverableFailure),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

fn decode_operation_kind(tag: u8) -> Result<InstantOperationKind, InstantError> {
    match tag {
        1 => Ok(InstantOperationKind::FenceAcquire),
        2 => Ok(InstantOperationKind::Begin),
        3 => Ok(InstantOperationKind::Connectivity),
        4 => Ok(InstantOperationKind::SegmentCommit),
        5 => Ok(InstantOperationKind::MultipartBind),
        6 => Ok(InstantOperationKind::MultipartRenew),
        7 => Ok(InstantOperationKind::PartClaim),
        8 => Ok(InstantOperationKind::PartDefer),
        9 => Ok(InstantOperationKind::PartProbe),
        10 => Ok(InstantOperationKind::PartVerify),
        11 => Ok(InstantOperationKind::SpoolEvict),
        12 => Ok(InstantOperationKind::FinalizeRequest),
        13 => Ok(InstantOperationKind::MultipartComplete),
        14 => Ok(InstantOperationKind::FinalizeDispatch),
        15 => Ok(InstantOperationKind::Publish),
        16 => Ok(InstantOperationKind::Callback),
        17 => Ok(InstantOperationKind::Cancel),
        18 => Ok(InstantOperationKind::RecoverableFailure),
        _ => Err(InstantError::MalformedJournalEncoding),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCall<T> {
    Committed(T),
    Duplicate(T),
    NotFound,
    AcknowledgementLost,
    Offline,
    Throttled { retry_after: Duration },
    Expired,
    Unavailable,
}

pub trait InstantMultipartPort: fmt::Debug {
    fn create_or_reconcile(
        &mut self,
        session_id: InstantSessionId,
        operation_id: InstantOperationId,
    ) -> Result<ProviderCall<InstantMultipartBinding>, InstantError>;
    fn inspect_upload(
        &mut self,
        session_id: InstantSessionId,
    ) -> Result<ProviderCall<InstantMultipartBinding>, InstantError>;
    fn renew(
        &mut self,
        binding: InstantMultipartBinding,
        operation_id: InstantOperationId,
    ) -> Result<ProviderCall<InstantMultipartBinding>, InstantError>;
    fn put_part(
        &mut self,
        binding: InstantMultipartBinding,
        ticket: &InstantPartUploadTicket,
        body: &mut ValidatedSegmentPayload,
    ) -> Result<ProviderCall<InstantPartReceipt>, InstantError>;
    fn inspect_part(
        &mut self,
        binding: InstantMultipartBinding,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<ProviderCall<InstantPartReceipt>, InstantError>;
    fn complete(
        &mut self,
        binding: InstantMultipartBinding,
        manifest: &InstantManifest,
        ordered_parts: &[InstantPartReceipt],
        operation_id: InstantOperationId,
    ) -> Result<ProviderCall<InstantMultipartCompleteReceipt>, InstantError>;
    fn inspect_complete(
        &mut self,
        binding: InstantMultipartBinding,
        manifest: &InstantManifest,
    ) -> Result<ProviderCall<InstantMultipartCompleteReceipt>, InstantError>;
    fn abort(
        &mut self,
        binding: InstantMultipartBinding,
        operation_id: InstantOperationId,
    ) -> Result<ProviderCall<()>, InstantError>;
    fn inspect_abort(
        &mut self,
        binding: InstantMultipartBinding,
    ) -> Result<ProviderCall<()>, InstantError>;
}

pub fn reconcile_instant_multipart_binding<P: InstantMultipartPort>(
    port: &mut P,
    session_id: InstantSessionId,
    operation_id: InstantOperationId,
) -> Result<InstantMultipartBinding, InstantError> {
    let call = port.create_or_reconcile(session_id, operation_id)?;
    let binding = match call {
        ProviderCall::Committed(binding) | ProviderCall::Duplicate(binding) => binding,
        ProviderCall::AcknowledgementLost => match port.inspect_upload(session_id)? {
            ProviderCall::Committed(binding) | ProviderCall::Duplicate(binding) => binding,
            _ => return Err(InstantError::AmbiguousMultipartBinding),
        },
        ProviderCall::Offline => return Err(InstantError::NetworkOffline),
        ProviderCall::Throttled { .. } => return Err(InstantError::ProviderThrottled),
        ProviderCall::Expired | ProviderCall::Unavailable | ProviderCall::NotFound => {
            return Err(InstantError::ProviderUnavailable);
        }
    };
    let binding = binding.validate()?;
    if binding.session_id != session_id {
        return Err(InstantError::SessionBindingMismatch);
    }
    Ok(binding)
}

pub fn renew_instant_multipart<P: InstantMultipartPort>(
    port: &mut P,
    session_id: InstantSessionId,
    current: InstantMultipartBinding,
    operation_id: InstantOperationId,
) -> Result<InstantMultipartBinding, InstantError> {
    let call = port.renew(current, operation_id)?;
    let renewed = match call {
        ProviderCall::Committed(binding) | ProviderCall::Duplicate(binding) => binding,
        ProviderCall::AcknowledgementLost => match port.inspect_upload(session_id)? {
            ProviderCall::Committed(binding) | ProviderCall::Duplicate(binding) => binding,
            _ => return Err(InstantError::AmbiguousMultipartBinding),
        },
        ProviderCall::Offline => return Err(InstantError::NetworkOffline),
        ProviderCall::Throttled { .. } => return Err(InstantError::ProviderThrottled),
        ProviderCall::Expired | ProviderCall::Unavailable | ProviderCall::NotFound => {
            return Err(InstantError::ProviderUnavailable);
        }
    };
    let renewed = renewed.validate()?;
    if current.session_id != session_id
        || renewed.session_id != session_id
        || renewed.upload_id != current.upload_id
        || renewed.generation <= current.generation
        || renewed.expires_at_ns <= current.expires_at_ns
        || renewed.minimum_part_bytes != current.minimum_part_bytes
        || renewed.maximum_part_bytes != current.maximum_part_bytes
        || renewed.maximum_parts != current.maximum_parts
    {
        return Err(InstantError::InvalidMultipartRenewal);
    }
    Ok(renewed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantUploadPolicy {
    pub maximum_concurrency: u16,
    pub maximum_attempts: u16,
    pub claim_lease: Duration,
    pub initial_backoff: Duration,
    pub maximum_backoff: Duration,
    pub maximum_retry_after: Duration,
}

impl InstantUploadPolicy {
    pub fn validate(self) -> Result<Self, InstantError> {
        if self.maximum_concurrency == 0
            || self.maximum_concurrency > 32
            || self.maximum_attempts == 0
            || self.maximum_attempts > 32
            || self.claim_lease.is_zero()
            || self.claim_lease > Duration::from_secs(15 * 60)
            || self.initial_backoff.is_zero()
            || self.maximum_backoff < self.initial_backoff
            || self.maximum_backoff > Duration::from_secs(60 * 60)
            || self.maximum_retry_after.is_zero()
            || self.maximum_retry_after > Duration::from_secs(24 * 60 * 60)
        {
            return Err(InstantError::InvalidUploadPolicy);
        }
        Ok(self)
    }

    fn retry_at(
        self,
        now_ns: u64,
        attempt: u16,
        retry_after: Option<Duration>,
    ) -> Result<u64, InstantError> {
        let delay = if let Some(retry_after) = retry_after {
            retry_after.min(self.maximum_retry_after)
        } else {
            let shift = u32::from(attempt.saturating_sub(1).min(30));
            self.initial_backoff
                .checked_mul(1_u32 << shift)
                .unwrap_or(self.maximum_backoff)
                .min(self.maximum_backoff)
        };
        let delay_ns = u64::try_from(delay.as_nanos()).map_err(|_| InstantError::ClockOverflow)?;
        now_ns
            .checked_add(delay_ns)
            .ok_or(InstantError::ClockOverflow)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantPartClaimPlan {
    pub index: u32,
    pub worker_id: InstantWorkerId,
    pub attempt: u16,
    pub lease_until_ns: u64,
    pub work_kind: InstantPartWorkKind,
    pub command: InstantJournalCommand,
}

/// A non-cloneable capability minted only after the durable journal contains
/// the exact in-flight claim.
pub struct InstantPartUploadTicket {
    session_id: InstantSessionId,
    index: u32,
    descriptor: InstantSegmentDescriptor,
    worker_id: InstantWorkerId,
    fence: InstantFence,
    attempt: u16,
    lease_until_ns: u64,
    work_kind: InstantPartWorkKind,
}

impl fmt::Debug for InstantPartUploadTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantPartUploadTicket")
            .field("index", &self.index)
            .field("attempt", &self.attempt)
            .field("lease_until_ns", &self.lease_until_ns)
            .finish_non_exhaustive()
    }
}

impl InstantPartUploadTicket {
    #[must_use]
    pub const fn session_id(&self) -> InstantSessionId {
        self.session_id
    }

    #[must_use]
    pub const fn descriptor(&self) -> &InstantSegmentDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn worker_id(&self) -> InstantWorkerId {
        self.worker_id
    }

    #[must_use]
    pub const fn fence(&self) -> InstantFence {
        self.fence
    }

    #[must_use]
    pub const fn attempt(&self) -> u16 {
        self.attempt
    }

    #[must_use]
    pub const fn work_kind(&self) -> InstantPartWorkKind {
        self.work_kind
    }
}

pub struct InstantUploadPlanner;

impl InstantUploadPlanner {
    pub fn plan_claim(
        snapshot: &InstantJournalSnapshot,
        policy: InstantUploadPolicy,
        worker_id: InstantWorkerId,
        now_ns: u64,
    ) -> Result<Option<InstantPartClaimPlan>, InstantError> {
        let policy = policy.validate()?;
        if snapshot.state == InstantJournalState::CapturingOffline {
            return Ok(None);
        }
        snapshot.require_live_upload_state()?;
        let binding = snapshot.multipart.ok_or(InstantError::MultipartNotBound)?;
        if binding.expires_at_ns <= now_ns {
            return Err(InstantError::MultipartExpired);
        }
        let active = snapshot
            .segments
            .values()
            .filter(|segment| {
                matches!(
                    segment.upload,
                    SegmentUploadJournalState::InFlight { lease_until_ns, .. }
                        if lease_until_ns > now_ns
                )
            })
            .count();
        if active >= usize::from(policy.maximum_concurrency) {
            return Ok(None);
        }
        for (index, segment) in &snapshot.segments {
            let prior_attempt = upload_attempt(&segment.upload);
            let (eligible, work_kind) = match segment.upload {
                SegmentUploadJournalState::Queued { eligible_at_ns, .. }
                | SegmentUploadJournalState::Deferred { eligible_at_ns, .. } => {
                    (eligible_at_ns <= now_ns, InstantPartWorkKind::Upload)
                }
                SegmentUploadJournalState::ProbeRequired { eligible_at_ns, .. } => {
                    (eligible_at_ns <= now_ns, InstantPartWorkKind::Probe)
                }
                SegmentUploadJournalState::InFlight {
                    lease_until_ns,
                    work_kind,
                    ..
                } => (lease_until_ns <= now_ns, work_kind),
                SegmentUploadJournalState::Verified(_) => (false, InstantPartWorkKind::Upload),
            };
            if !eligible {
                continue;
            }
            let attempt = prior_attempt
                .checked_add(1)
                .ok_or(InstantError::UploadAttemptsExhausted)?;
            if attempt > policy.maximum_attempts {
                return Err(InstantError::UploadAttemptsExhausted);
            }
            let lease_ns = u64::try_from(policy.claim_lease.as_nanos())
                .map_err(|_| InstantError::ClockOverflow)?;
            let lease_until_ns = now_ns
                .checked_add(lease_ns)
                .ok_or(InstantError::ClockOverflow)?;
            return Ok(Some(InstantPartClaimPlan {
                index: *index,
                worker_id,
                attempt,
                lease_until_ns,
                work_kind,
                command: InstantJournalCommand::ClaimPart {
                    index: *index,
                    worker_id,
                    work_kind,
                    attempt,
                    lease_until_ns,
                },
            }));
        }
        Ok(None)
    }

    pub fn activate_ticket(
        snapshot: &InstantJournalSnapshot,
        plan: InstantPartClaimPlan,
    ) -> Result<InstantPartUploadTicket, InstantError> {
        let segment = snapshot
            .segments
            .get(&plan.index)
            .ok_or(InstantError::UnknownSegment(plan.index))?;
        match segment.upload {
            SegmentUploadJournalState::InFlight {
                worker_id,
                fence,
                work_kind,
                attempt,
                lease_until_ns,
            } if worker_id == plan.worker_id
                && fence == snapshot.fence
                && work_kind == plan.work_kind
                && attempt == plan.attempt
                && lease_until_ns == plan.lease_until_ns =>
            {
                Ok(InstantPartUploadTicket {
                    session_id: snapshot.session_id,
                    index: plan.index,
                    descriptor: segment.descriptor.clone(),
                    worker_id,
                    fence,
                    work_kind,
                    attempt,
                    lease_until_ns,
                })
            }
            _ => Err(InstantError::StaleUploadClaim),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstantPartResolution {
    Journal(Box<InstantJournalCommand>),
    RenewRequired,
}

pub fn upload_instant_part<P: InstantMultipartPort>(
    port: &mut P,
    binding: InstantMultipartBinding,
    snapshot: &InstantJournalSnapshot,
    ticket: InstantPartUploadTicket,
    body: &mut ValidatedSegmentPayload,
    policy: InstantUploadPolicy,
    now_ns: u64,
) -> Result<InstantPartResolution, InstantError> {
    let policy = policy.validate()?;
    if snapshot.session_id != ticket.session_id
        || snapshot.fence != ticket.fence
        || snapshot.multipart != Some(binding)
    {
        return Err(InstantError::StaleUploadClaim);
    }
    let segment = snapshot
        .segments
        .get(&ticket.index)
        .ok_or(InstantError::StaleUploadClaim)?;
    if !matches!(
        segment.upload,
        SegmentUploadJournalState::InFlight {
            worker_id,
            fence,
            work_kind,
            attempt,
            lease_until_ns,
        } if worker_id == ticket.worker_id
            && fence == ticket.fence
            && work_kind == ticket.work_kind
            && attempt == ticket.attempt
            && lease_until_ns == ticket.lease_until_ns
    ) {
        return Err(InstantError::StaleUploadClaim);
    }
    if binding.expires_at_ns <= now_ns || ticket.lease_until_ns <= now_ns {
        return Ok(InstantPartResolution::RenewRequired);
    }
    let result = if ticket.work_kind == InstantPartWorkKind::Probe {
        match port.inspect_part(binding, &ticket.descriptor)? {
            ProviderCall::NotFound => port.put_part(binding, &ticket, body)?,
            result => result,
        }
    } else {
        port.put_part(binding, &ticket, body)?
    };
    resolve_part_provider_call(
        port,
        binding,
        &ticket,
        policy,
        now_ns,
        ticket.work_kind == InstantPartWorkKind::Probe,
        result,
    )
}

fn resolve_part_provider_call<P: InstantMultipartPort>(
    port: &mut P,
    binding: InstantMultipartBinding,
    ticket: &InstantPartUploadTicket,
    policy: InstantUploadPolicy,
    now_ns: u64,
    probe_only: bool,
    result: ProviderCall<InstantPartReceipt>,
) -> Result<InstantPartResolution, InstantError> {
    let verify = |receipt: InstantPartReceipt| -> Result<InstantPartResolution, InstantError> {
        let receipt = receipt.validate_for(binding, &ticket.descriptor)?;
        Ok(InstantPartResolution::Journal(Box::new(
            InstantJournalCommand::VerifyPart {
                index: ticket.index,
                receipt,
            },
        )))
    };
    match result {
        ProviderCall::Committed(receipt) | ProviderCall::Duplicate(receipt) => verify(receipt),
        ProviderCall::AcknowledgementLost => {
            match port.inspect_part(binding, &ticket.descriptor)? {
                ProviderCall::Committed(receipt) | ProviderCall::Duplicate(receipt) => {
                    verify(receipt)
                }
                ProviderCall::Expired => Ok(InstantPartResolution::RenewRequired),
                ProviderCall::NotFound => Ok(InstantPartResolution::Journal(Box::new(
                    InstantJournalCommand::DeferPart {
                        index: ticket.index,
                        attempt: ticket.attempt,
                        eligible_at_ns: policy.retry_at(now_ns, ticket.attempt, None)?,
                        reason: UploadDeferReason::LostAcknowledgement,
                    },
                ))),
                ProviderCall::Offline
                | ProviderCall::Unavailable
                | ProviderCall::AcknowledgementLost
                | ProviderCall::Throttled { .. } => Ok(InstantPartResolution::Journal(Box::new(
                    InstantJournalCommand::RequirePartProbe {
                        index: ticket.index,
                        attempt: ticket.attempt,
                        eligible_at_ns: policy.retry_at(now_ns, ticket.attempt, None)?,
                    },
                ))),
            }
        }
        ProviderCall::Offline => unresolved_part(
            ticket,
            policy.retry_at(now_ns, ticket.attempt, None)?,
            probe_only,
            UploadDeferReason::Offline,
        ),
        ProviderCall::Throttled { retry_after } => unresolved_part(
            ticket,
            policy.retry_at(now_ns, ticket.attempt, Some(retry_after))?,
            probe_only,
            UploadDeferReason::Throttled,
        ),
        ProviderCall::Unavailable => unresolved_part(
            ticket,
            policy.retry_at(now_ns, ticket.attempt, None)?,
            probe_only,
            UploadDeferReason::ProviderUnavailable,
        ),
        ProviderCall::NotFound => unresolved_part(
            ticket,
            policy.retry_at(now_ns, ticket.attempt, None)?,
            probe_only,
            UploadDeferReason::LostAcknowledgement,
        ),
        ProviderCall::Expired => Ok(InstantPartResolution::RenewRequired),
    }
}

fn unresolved_part(
    ticket: &InstantPartUploadTicket,
    eligible_at_ns: u64,
    probe_only: bool,
    reason: UploadDeferReason,
) -> Result<InstantPartResolution, InstantError> {
    let command = if probe_only {
        InstantJournalCommand::RequirePartProbe {
            index: ticket.index,
            attempt: ticket.attempt,
            eligible_at_ns,
        }
    } else {
        InstantJournalCommand::DeferPart {
            index: ticket.index,
            attempt: ticket.attempt,
            eligible_at_ns,
            reason,
        }
    };
    Ok(InstantPartResolution::Journal(Box::new(command)))
}

pub fn complete_instant_multipart<P: InstantMultipartPort>(
    port: &mut P,
    snapshot: &InstantJournalSnapshot,
    operation_id: InstantOperationId,
) -> Result<InstantMultipartCompleteReceipt, InstantError> {
    snapshot.require_state(&[InstantJournalState::Finalizing])?;
    let binding = snapshot.multipart.ok_or(InstantError::MultipartNotBound)?;
    let manifest = snapshot
        .manifest
        .as_ref()
        .ok_or(InstantError::ManifestMissing)?;
    let parts = snapshot
        .segments
        .values()
        .map(|segment| match segment.upload {
            SegmentUploadJournalState::Verified(receipt) => Ok(receipt),
            _ => Err(InstantError::SegmentsNotRemotelyVerified),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = port.complete(binding, manifest, &parts, operation_id)?;
    let receipt = match result {
        ProviderCall::Committed(receipt) | ProviderCall::Duplicate(receipt) => receipt,
        ProviderCall::AcknowledgementLost => match port.inspect_complete(binding, manifest)? {
            ProviderCall::Committed(receipt) | ProviderCall::Duplicate(receipt) => receipt,
            _ => return Err(InstantError::AmbiguousMultipartComplete),
        },
        ProviderCall::Expired => return Err(InstantError::MultipartExpired),
        ProviderCall::Offline => return Err(InstantError::NetworkOffline),
        ProviderCall::Throttled { .. } => return Err(InstantError::ProviderThrottled),
        ProviderCall::Unavailable | ProviderCall::NotFound => {
            return Err(InstantError::ProviderUnavailable);
        }
    };
    snapshot.validate_multipart_complete(receipt)?;
    Ok(receipt)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeProviderCall {
    Published(InstantFinalizeReceipt),
    Duplicate(InstantFinalizeReceipt),
    Pending,
    AcknowledgementLost,
    StaleGeneration,
    Unavailable,
}

pub trait InstantFinalizePort: fmt::Debug {
    fn reconcile(
        &mut self,
        request: &InstantFinalizeRequest,
        operation_id: InstantOperationId,
    ) -> Result<FinalizeProviderCall, InstantError>;
    fn inspect(
        &mut self,
        request: &InstantFinalizeRequest,
    ) -> Result<FinalizeProviderCall, InstantError>;
    fn cancel_job(
        &mut self,
        session_id: InstantSessionId,
        fence: InstantFence,
        operation_id: InstantOperationId,
    ) -> Result<ProviderCall<()>, InstantError>;
    fn inspect_cancel_job(
        &mut self,
        session_id: InstantSessionId,
    ) -> Result<ProviderCall<()>, InstantError>;
}

pub fn reconcile_instant_finalize<P: InstantFinalizePort>(
    port: &mut P,
    request: &InstantFinalizeRequest,
    operation_id: InstantOperationId,
) -> Result<Option<InstantFinalizeReceipt>, InstantError> {
    let call = port.reconcile(request, operation_id)?;
    let receipt = match call {
        FinalizeProviderCall::Published(receipt) | FinalizeProviderCall::Duplicate(receipt) => {
            Some(receipt)
        }
        FinalizeProviderCall::AcknowledgementLost => match port.inspect(request)? {
            FinalizeProviderCall::Published(receipt) | FinalizeProviderCall::Duplicate(receipt) => {
                Some(receipt)
            }
            FinalizeProviderCall::Pending | FinalizeProviderCall::AcknowledgementLost => None,
            FinalizeProviderCall::StaleGeneration => {
                return Err(InstantError::StaleJobGeneration);
            }
            FinalizeProviderCall::Unavailable => return Err(InstantError::ProviderUnavailable),
        },
        FinalizeProviderCall::Pending => None,
        FinalizeProviderCall::StaleGeneration => return Err(InstantError::StaleJobGeneration),
        FinalizeProviderCall::Unavailable => return Err(InstantError::ProviderUnavailable),
    };
    if let Some(receipt) = receipt {
        validate_finalize_receipt(request, receipt)?;
    }
    Ok(receipt)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantCancellationReport {
    pub upload_aborted: bool,
    pub finalize_job_cancelled: bool,
    pub spool_wiped: bool,
    pub retry_required: bool,
}

/// Performs external cleanup only after the durable journal has sealed its
/// tombstone. Every cleanup branch is attempted even when another branch must
/// be retried, so disk and provider resources do not depend on one another.
pub fn reconcile_instant_tombstone<M, F, S>(
    snapshot: &InstantJournalSnapshot,
    multipart_port: &mut M,
    finalize_port: &mut F,
    spool: InstantSpool<S>,
    abort_operation: InstantOperationId,
    cancel_job_operation: InstantOperationId,
) -> Result<InstantCancellationReport, InstantError>
where
    M: InstantMultipartPort,
    F: InstantFinalizePort,
    S: PrivateSpoolPort,
{
    snapshot.require_state(&[InstantJournalState::Tombstoned])?;
    let mut retry_required = false;
    let upload_aborted = if let Some(binding) = snapshot.multipart {
        match multipart_port.abort(binding, abort_operation) {
            Ok(ProviderCall::Committed(()) | ProviderCall::Duplicate(())) => true,
            Ok(ProviderCall::AcknowledgementLost) => match multipart_port.inspect_abort(binding) {
                Ok(ProviderCall::Committed(()) | ProviderCall::Duplicate(())) => true,
                _ => {
                    retry_required = true;
                    false
                }
            },
            Ok(_) | Err(_) => {
                retry_required = true;
                false
            }
        }
    } else {
        true
    };
    let finalize_job_cancelled =
        match finalize_port.cancel_job(snapshot.session_id, snapshot.fence, cancel_job_operation) {
            Ok(ProviderCall::Committed(()) | ProviderCall::Duplicate(())) => true,
            Ok(ProviderCall::AcknowledgementLost) => {
                match finalize_port.inspect_cancel_job(snapshot.session_id) {
                    Ok(ProviderCall::Committed(()) | ProviderCall::Duplicate(())) => true,
                    _ => {
                        retry_required = true;
                        false
                    }
                }
            }
            Ok(_) | Err(_) => {
                retry_required = true;
                false
            }
        };
    let spool_wiped = match spool.wipe() {
        Ok(()) => true,
        Err(_) => {
            retry_required = true;
            false
        }
    };
    Ok(InstantCancellationReport {
        upload_aborted,
        finalize_job_cancelled,
        spool_wiped,
        retry_required,
    })
}

fn validate_finalize_receipt(
    request: &InstantFinalizeRequest,
    receipt: InstantFinalizeReceipt,
) -> Result<(), InstantError> {
    if receipt.request_digest != request.digest
        || receipt.job_id != request.job_id
        || receipt.job_generation != request.job_generation
        || receipt.object != request.multipart.object
        || receipt.playable_master.object_id != request.multipart.object.object_id
        || receipt.playable_master.immutable_digest != request.multipart.object.object_version
        || !receipt.playable_master.distribution_eligible
    {
        return Err(InstantError::InvalidFinalizeReceipt);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivativePreference {
    PreferManagedMedia,
    PreferNativeGstreamer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivativeCapabilities {
    pub managed_media_available: bool,
    pub managed_media_accepts_master: bool,
    pub native_gstreamer_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivativePlan {
    ManagedMedia { source: PlayableMasterIdentity },
    NativeGstreamer { source: PlayableMasterIdentity },
}

pub fn choose_derivative_plan(
    receipt: InstantFinalizeReceipt,
    capabilities: DerivativeCapabilities,
    preference: DerivativePreference,
) -> Result<DerivativePlan, InstantError> {
    if !receipt.playable_master.distribution_eligible {
        return Err(InstantError::DistributionMasterUnavailable);
    }
    match preference {
        DerivativePreference::PreferManagedMedia
            if capabilities.managed_media_available
                && capabilities.managed_media_accepts_master =>
        {
            Ok(DerivativePlan::ManagedMedia {
                source: receipt.playable_master,
            })
        }
        DerivativePreference::PreferNativeGstreamer if capabilities.native_gstreamer_available => {
            Ok(DerivativePlan::NativeGstreamer {
                source: receipt.playable_master,
            })
        }
        _ if capabilities.managed_media_available && capabilities.managed_media_accepts_master => {
            Ok(DerivativePlan::ManagedMedia {
                source: receipt.playable_master,
            })
        }
        _ if capabilities.native_gstreamer_available => Ok(DerivativePlan::NativeGstreamer {
            source: receipt.playable_master,
        }),
        _ => Err(InstantError::DerivativeCapabilityUnavailable),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantPublicErrorCode {
    LocalStorageFull,
    LocalStorageUnavailable,
    NetworkOffline,
    UploadDelayed,
    UploadExpired,
    FinalizeDelayed,
    RecordingRecoveryRequired,
    RecordingCancelled,
    RecordingFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantReadiness {
    Recording,
    LocallyRecoverable,
    Uploading,
    Finalizing,
    ShareReady,
    Cancelled,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantProgress {
    pub version: u16,
    pub readiness: InstantReadiness,
    pub segment_count: u32,
    pub verified_segment_count: u32,
    pub retained_spool_bytes: u64,
    pub total_media_bytes: u64,
    pub upload_basis_points: u16,
    pub network_available: bool,
    pub retrying: bool,
    pub public_error: Option<InstantPublicErrorCode>,
}

impl InstantProgress {
    pub fn from_snapshot(
        snapshot: &InstantJournalSnapshot,
        retained_spool_bytes: u64,
        error: Option<InstantPublicErrorCode>,
    ) -> Result<Self, InstantError> {
        let segment_count =
            u32::try_from(snapshot.segments.len()).map_err(|_| InstantError::JournalCorrupt)?;
        let verified_segment_count = u32::try_from(
            snapshot
                .segments
                .values()
                .filter(|segment| matches!(segment.upload, SegmentUploadJournalState::Verified(_)))
                .count(),
        )
        .map_err(|_| InstantError::JournalCorrupt)?;
        let total_media_bytes = snapshot
            .segments
            .values()
            .try_fold(0_u64, |total, segment| {
                total
                    .checked_add(segment.descriptor.bytes)
                    .ok_or(InstantError::JournalCorrupt)
            })?;
        let upload_basis_points = if segment_count == 0 {
            0
        } else {
            u16::try_from(
                u64::from(verified_segment_count)
                    .saturating_mul(10_000)
                    .checked_div(u64::from(segment_count))
                    .unwrap_or(0),
            )
            .map_err(|_| InstantError::JournalCorrupt)?
        };
        let retrying = snapshot.segments.values().any(|segment| {
            matches!(
                segment.upload,
                SegmentUploadJournalState::Deferred { .. }
                    | SegmentUploadJournalState::ProbeRequired { .. }
            )
        });
        let readiness = match snapshot.state {
            InstantJournalState::Created | InstantJournalState::CapturingOnline => {
                InstantReadiness::Recording
            }
            InstantJournalState::CapturingOffline => InstantReadiness::LocallyRecoverable,
            InstantJournalState::Finalizing if snapshot.multipart_complete.is_none() => {
                InstantReadiness::Uploading
            }
            InstantJournalState::Finalizing => InstantReadiness::Finalizing,
            InstantJournalState::Ready => InstantReadiness::ShareReady,
            InstantJournalState::Tombstoned => InstantReadiness::Cancelled,
            InstantJournalState::RecoverableFailure => InstantReadiness::RecoveryRequired,
        };
        Ok(Self {
            version: INSTANT_PROGRESS_VERSION,
            readiness,
            segment_count,
            verified_segment_count,
            retained_spool_bytes,
            total_media_bytes,
            upload_basis_points,
            network_available: snapshot.state != InstantJournalState::CapturingOffline,
            retrying,
            public_error: error,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantResourceBudget {
    pub maximum_spool_bytes: u64,
    pub maximum_inflight_upload_bytes: u64,
    pub maximum_inflight_parts: u16,
    pub share_target: Duration,
}

impl InstantResourceBudget {
    pub fn validate(self) -> Result<Self, InstantError> {
        if self.maximum_spool_bytes == 0
            || self.maximum_inflight_upload_bytes == 0
            || self.maximum_inflight_parts == 0
            || self.maximum_inflight_parts > 32
            || self.share_target.is_zero()
        {
            return Err(InstantError::InvalidResourceBudget);
        }
        Ok(self)
    }

    pub fn simulate_time_to_share(
        self,
        remaining_bytes: u64,
        sustained_bytes_per_second: u64,
        finalize_latency: Duration,
    ) -> Result<Duration, InstantError> {
        self.validate()?;
        if sustained_bytes_per_second == 0 {
            return Err(InstantError::InvalidThroughput);
        }
        let upload_millis = remaining_bytes
            .saturating_mul(1_000)
            .saturating_add(sustained_bytes_per_second - 1)
            / sustained_bytes_per_second;
        Duration::from_millis(upload_millis)
            .checked_add(finalize_latency)
            .ok_or(InstantError::ClockOverflow)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InstantError {
    #[error("Instant session identity is invalid")]
    InvalidSessionId,
    #[error("Instant operation identity is invalid")]
    InvalidOperationId,
    #[error("Instant worker identity is invalid")]
    InvalidWorkerId,
    #[error("Instant upload identity is invalid")]
    InvalidUploadId,
    #[error("Instant publication identity is invalid")]
    InvalidPublicationId,
    #[error("Instant object identity is invalid")]
    InvalidObjectId,
    #[error("Instant job identity is invalid")]
    InvalidJobId,
    #[error("runtime spool key handle is invalid")]
    InvalidSpoolKeyHandle,
    #[error("SHA-256 checksum is invalid")]
    InvalidChecksum,
    #[error("segment metadata is invalid")]
    InvalidSegment,
    #[error("segment track metadata is invalid")]
    InvalidTrackMetadata,
    #[error("segment track codec does not match its role")]
    TrackCodecMismatch,
    #[error("segment tracks are not aligned to the fragment boundary")]
    TrackAlignmentMismatch,
    #[error("segment is missing the screen-video track")]
    MissingScreenTrack,
    #[error("segment belongs to another Instant session")]
    SessionBindingMismatch,
    #[error("segment {0} conflicts with an immutable segment")]
    SegmentConflict(u32),
    #[error("segment sequence is discontinuous: expected {expected_index}, found {found_index}")]
    SegmentContinuity {
        expected_index: u32,
        found_index: u32,
    },
    #[error("Instant manifest requires at least one segment")]
    NoSegments,
    #[error("Instant distribution-master pipeline request is invalid")]
    InvalidPipelineRequest,
    #[error("Media-compatible distribution-master capability is unavailable")]
    DistributionMasterUnavailable,
    #[error("segment body length does not match immutable metadata")]
    PayloadLengthMismatch,
    #[error("segment body SHA-256 does not match immutable metadata")]
    PayloadChecksumMismatch,
    #[error("segment body returned an empty or oversized chunk")]
    InvalidPayloadChunk,
    #[error("encrypted, authenticated, atomic private spool is unavailable")]
    SecureSpoolUnavailable,
    #[error("runtime spool key is unavailable")]
    SpoolKeyUnavailable,
    #[error("local spool quota is invalid")]
    InvalidSpoolQuota,
    #[error("local spool quota is exhausted")]
    SpoolQuotaExceeded,
    #[error("local spool storage is full")]
    SpoolDiskFull,
    #[error("local spool entry is missing")]
    SpoolEntryMissing,
    #[error("local spool data is corrupt or truncated")]
    SpoolCorrupt,
    #[error("local spool receipt is invalid")]
    InvalidSpoolReceipt,
    #[error("remote durability has not been proven")]
    RemoteDurabilityUnproven,
    #[error("operation was already applied")]
    OperationAlreadyApplied,
    #[error("journal fence is invalid")]
    InvalidFence,
    #[error("journal revision is stale")]
    StaleJournal,
    #[error("journal worker fence is stale")]
    StaleFence,
    #[error("Instant journal is corrupt or overflowed")]
    JournalCorrupt,
    #[error("Instant journal encoding is malformed")]
    MalformedJournalEncoding,
    #[error("Instant journal encoding exceeds the bounded size")]
    JournalEncodingTooLarge,
    #[error("Instant journal encoding version is unsupported")]
    UnsupportedJournalVersion,
    #[error("Instant journal is missing")]
    JournalMissing,
    #[error("Instant journal commit outcome is ambiguous")]
    AmbiguousJournalCommit,
    #[error("Instant journal cannot perform this operation while {0:?}")]
    InvalidState(InstantJournalState),
    #[error("operation key was reused for a different command")]
    OperationKeyConflict,
    #[error("operation receipt capacity is exhausted")]
    OperationReceiptCapacityExceeded,
    #[error("multipart upload binding is invalid")]
    InvalidMultipartBinding,
    #[error("multipart upload binding conflicts with the journal")]
    MultipartBindingConflict,
    #[error("multipart upload binding outcome remains ambiguous")]
    AmbiguousMultipartBinding,
    #[error("multipart upload is not bound")]
    MultipartNotBound,
    #[error("multipart upload renewal is invalid")]
    InvalidMultipartRenewal,
    #[error("multipart upload has expired")]
    MultipartExpired,
    #[error("multipart part claim is invalid")]
    InvalidUploadClaim,
    #[error("segment {0} is not claimable")]
    PartNotClaimable(u32),
    #[error("upload claim is stale")]
    StaleUploadClaim,
    #[error("retry schedule is invalid")]
    InvalidRetrySchedule,
    #[error("segment {0} is not in the journal")]
    UnknownSegment(u32),
    #[error("multipart part receipt is invalid")]
    InvalidPartReceipt,
    #[error("segment does not satisfy multipart part size/count limits")]
    MultipartPartSizeMismatch,
    #[error("multipart part receipt conflicts for segment {0}")]
    PartReceiptConflict(u32),
    #[error("not every segment is remotely verified")]
    SegmentsNotRemotelyVerified,
    #[error("Instant manifest is missing")]
    ManifestMissing,
    #[error("multipart completion is missing")]
    MultipartNotComplete,
    #[error("multipart completion receipt is invalid")]
    InvalidMultipartCompleteReceipt,
    #[error("multipart completion conflicts with the journal")]
    MultipartCompleteConflict,
    #[error("multipart completion outcome remains ambiguous")]
    AmbiguousMultipartComplete,
    #[error("finalize request is invalid")]
    InvalidFinalizeRequest,
    #[error("finalize request has not been durably sealed")]
    FinalizeRequestNotSealed,
    #[error("finalize receipt is invalid")]
    InvalidFinalizeReceipt,
    #[error("D1/job generation is invalid")]
    InvalidJobGeneration,
    #[error("D1/job generation is stale")]
    StaleJobGeneration,
    #[error("a different immutable recording is already published")]
    PublishConflict,
    #[error("a terminal tombstone prevents resurrection")]
    TombstoneSealed,
    #[error("upload retry policy is invalid")]
    InvalidUploadPolicy,
    #[error("upload attempts are exhausted")]
    UploadAttemptsExhausted,
    #[error("monotonic clock arithmetic overflowed")]
    ClockOverflow,
    #[error("network is offline")]
    NetworkOffline,
    #[error("provider throttled the request")]
    ProviderThrottled,
    #[error("provider is unavailable")]
    ProviderUnavailable,
    #[error("no derivative engine can consume the finalized master")]
    DerivativeCapabilityUnavailable,
    #[error("Instant resource budget is invalid")]
    InvalidResourceBudget,
    #[error("throughput must be non-zero")]
    InvalidThroughput,
}

fn upload_attempt(state: &SegmentUploadJournalState) -> u16 {
    match state {
        SegmentUploadJournalState::Queued { attempt, .. }
        | SegmentUploadJournalState::InFlight { attempt, .. }
        | SegmentUploadJournalState::Deferred { attempt, .. }
        | SegmentUploadJournalState::ProbeRequired { attempt, .. } => *attempt,
        SegmentUploadJournalState::Verified(_) => 0,
    }
}

fn deterministic_job_id(
    session_id: InstantSessionId,
    manifest: Sha256Digest,
) -> Result<InstantJobId, InstantError> {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(b"frame.instant.job.v1\0");
    bytes.extend_from_slice(&session_id.canonical_bytes());
    bytes.extend_from_slice(&manifest.canonical_bytes());
    let digest = strong_sha256(&bytes).canonical_bytes();
    let mut identity = [0_u8; 16];
    identity.copy_from_slice(&digest[..16]);
    InstantJobId::from_csprng(identity)
}

fn append_spool_receipt(bytes: &mut Vec<u8>, receipt: SpoolCommitReceipt) {
    bytes.extend_from_slice(&receipt.segment_index.to_be_bytes());
    bytes.extend_from_slice(&receipt.segment_identity.canonical_bytes());
    bytes.extend_from_slice(&receipt.bytes.to_be_bytes());
    bytes.extend_from_slice(&receipt.ciphertext_integrity.canonical_bytes());
    bytes.push(u8::from(receipt.durable));
}

fn append_multipart_binding(bytes: &mut Vec<u8>, binding: InstantMultipartBinding) {
    bytes.extend_from_slice(&binding.session_id.canonical_bytes());
    bytes.extend_from_slice(&binding.upload_id.canonical_bytes());
    bytes.extend_from_slice(&binding.expires_at_ns.to_be_bytes());
    bytes.extend_from_slice(&binding.generation.to_be_bytes());
    bytes.extend_from_slice(&binding.minimum_part_bytes.to_be_bytes());
    bytes.extend_from_slice(&binding.maximum_part_bytes.to_be_bytes());
    bytes.extend_from_slice(&binding.maximum_parts.to_be_bytes());
}

fn append_object_manifest(bytes: &mut Vec<u8>, object: InstantObjectManifest) {
    bytes.extend_from_slice(&object.object_id.canonical_bytes());
    bytes.extend_from_slice(&object.object_version.canonical_bytes());
    bytes.extend_from_slice(&object.instant_manifest.canonical_bytes());
    bytes.extend_from_slice(&object.ordered_parts_digest.canonical_bytes());
    bytes.extend_from_slice(&object.bytes.to_be_bytes());
}

fn append_complete_receipt(bytes: &mut Vec<u8>, receipt: InstantMultipartCompleteReceipt) {
    bytes.extend_from_slice(&receipt.session_id.canonical_bytes());
    bytes.extend_from_slice(&receipt.upload_id.canonical_bytes());
    bytes.extend_from_slice(&receipt.upload_generation.to_be_bytes());
    bytes.extend_from_slice(&receipt.manifest_digest.canonical_bytes());
    bytes.extend_from_slice(&receipt.ordered_parts_digest.canonical_bytes());
    append_object_manifest(bytes, receipt.object);
}

fn append_finalize_receipt(bytes: &mut Vec<u8>, receipt: InstantFinalizeReceipt) {
    bytes.extend_from_slice(&receipt.request_digest.canonical_bytes());
    bytes.extend_from_slice(&receipt.publication_id.canonical_bytes());
    bytes.extend_from_slice(&receipt.job_id.canonical_bytes());
    bytes.extend_from_slice(&receipt.job_generation.to_be_bytes());
    append_object_manifest(bytes, receipt.object);
    bytes.extend_from_slice(&receipt.playable_master.object_id.canonical_bytes());
    bytes.extend_from_slice(&receipt.playable_master.immutable_digest.canonical_bytes());
    bytes.push(u8::from(receipt.playable_master.distribution_eligible));
}

fn operation_kind_tag(kind: InstantOperationKind) -> u8 {
    match kind {
        InstantOperationKind::FenceAcquire => 1,
        InstantOperationKind::Begin => 2,
        InstantOperationKind::Connectivity => 3,
        InstantOperationKind::SegmentCommit => 4,
        InstantOperationKind::MultipartBind => 5,
        InstantOperationKind::MultipartRenew => 6,
        InstantOperationKind::PartClaim => 7,
        InstantOperationKind::PartDefer => 8,
        InstantOperationKind::PartProbe => 9,
        InstantOperationKind::PartVerify => 10,
        InstantOperationKind::SpoolEvict => 11,
        InstantOperationKind::FinalizeRequest => 12,
        InstantOperationKind::MultipartComplete => 13,
        InstantOperationKind::FinalizeDispatch => 14,
        InstantOperationKind::Publish => 15,
        InstantOperationKind::Callback => 16,
        InstantOperationKind::Cancel => 17,
        InstantOperationKind::RecoverableFailure => 18,
    }
}

fn defer_reason_tag(reason: UploadDeferReason) -> u8 {
    match reason {
        UploadDeferReason::Offline => 1,
        UploadDeferReason::Throttled => 2,
        UploadDeferReason::ProviderUnavailable => 3,
        UploadDeferReason::UploadExpired => 4,
        UploadDeferReason::LostAcknowledgement => 5,
    }
}

fn journal_state_tag(state: InstantJournalState) -> u8 {
    match state {
        InstantJournalState::Created => 1,
        InstantJournalState::CapturingOnline => 2,
        InstantJournalState::CapturingOffline => 3,
        InstantJournalState::Finalizing => 4,
        InstantJournalState::Ready => 5,
        InstantJournalState::Tombstoned => 6,
        InstantJournalState::RecoverableFailure => 7,
    }
}

#[derive(Clone)]
struct Sha256State {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256State {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    fn new() -> Self {
        Self {
            state: Self::INITIAL,
            block: [0; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.total_len = self.total_len.saturating_add(bytes.len() as u64);
        if self.block_len != 0 {
            let needed = 64 - self.block_len;
            let take = needed.min(bytes.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&bytes[..take]);
            self.block_len += take;
            bytes = &bytes[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
        while bytes.len() >= 64 {
            let mut block = [0_u8; 64];
            block.copy_from_slice(&bytes[..64]);
            self.compress(&block);
            bytes = &bytes[64..];
        }
        if !bytes.is_empty() {
            self.block[..bytes.len()].copy_from_slice(bytes);
            self.block_len = bytes.len();
        }
    }

    fn finalize(mut self) -> Sha256Digest {
        let bit_len = self.total_len.saturating_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
            self.block_len = 0;
        }
        self.block[self.block_len..56].fill(0);
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);
        let mut digest = [0_u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        Sha256Digest(digest)
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut schedule = [0_u32; 64];
        for (index, chunk) in block.chunks_exact(4).take(16).enumerate() {
            schedule[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e
                .rotate_right(6)
                .wrapping_xor(e.rotate_right(11))
                .wrapping_xor(e.rotate_right(25));
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let sum0 = a
                .rotate_right(2)
                .wrapping_xor(a.rotate_right(13))
                .wrapping_xor(a.rotate_right(22));
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

/// Computes SHA-256 without delegating media identity to a provider adapter.
#[must_use]
pub fn strong_sha256(bytes: &[u8]) -> Sha256Digest {
    let mut state = Sha256State::new();
    state.update(bytes);
    state.finalize()
}

trait WrappingXor {
    fn wrapping_xor(self, other: Self) -> Self;
}

impl WrappingXor for u32 {
    fn wrapping_xor(self, other: Self) -> Self {
        self ^ other
    }
}
