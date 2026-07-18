use std::{collections::HashMap, fmt};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    AudioCodec, CancellationCapability, ContainerFormat, ExecutionFailureClass, MediaExecutorKind,
    MediaInput, MediaJobKind, ProgressCapability, VideoCodec, media_job_catalog,
};

pub const MEDIA_SERVICE_CATALOG_VERSION: u16 = 1;
pub const MEDIA_TRANSFORM_PROFILE_SCHEMA_VERSION: u16 = 1;
pub const MEDIA_JOB_JOURNAL_SCHEMA_VERSION: u16 = 1;
pub const CAP_MEDIA_REFERENCE_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaInputRole {
    SourceOriginal,
    RecordingSegments,
    DistributionMaster,
    ExtractedAudio,
    EditTimeline,
    Captions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOutputRole {
    Preview,
    Thumbnail,
    Spritesheet,
    ExtractedAudio,
    ProbeManifest,
    DistributionMaster,
    AnimatedPreview,
    NormalizedAudio,
    RepairedMedia,
    MuxedMedia,
    Waveform,
    Composition,
    NormalizedMedia,
    Captions,
    AiMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaExecutionDisposition {
    HybridManagedNative,
    NativeOnly,
    ExternalProviderAdapter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyPolicy {
    DeterministicImmutableArtifact,
    DeterministicManifest,
    ProviderRequestAndResultDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    Denied,
    PrivateObjectBindingsOnly,
    DeclaredProviderAdapterOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaSandboxLimits {
    pub max_source_bytes: u64,
    pub max_duration_ms: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_decoded_bytes: u64,
    pub max_frames: u64,
    pub max_tracks: u16,
    pub max_memory_bytes: u64,
    pub max_scratch_bytes: u64,
    pub max_cpu_millis: u64,
    pub max_gpu_millis: u64,
    pub max_output_bytes: u64,
    pub max_cost_microunits: u64,
    pub network: NetworkPolicy,
}

const LIGHT_NATIVE: MediaSandboxLimits = MediaSandboxLimits {
    max_source_bytes: 2_000_000_000,
    max_duration_ms: 14_400_000,
    max_width: 7_680,
    max_height: 4_320,
    max_decoded_bytes: 64_000_000_000,
    max_frames: 1_300_000,
    max_tracks: 32,
    max_memory_bytes: 1_073_741_824,
    max_scratch_bytes: 4_000_000_000,
    max_cpu_millis: 900_000,
    max_gpu_millis: 900_000,
    max_output_bytes: 512_000_000,
    max_cost_microunits: 10_000_000,
    network: NetworkPolicy::Denied,
};

const HEAVY_NATIVE: MediaSandboxLimits = MediaSandboxLimits {
    max_source_bytes: 20_000_000_000,
    max_duration_ms: 43_200_000,
    max_width: 7_680,
    max_height: 4_320,
    max_decoded_bytes: 512_000_000_000,
    max_frames: 4_000_000,
    max_tracks: 64,
    max_memory_bytes: 8_589_934_592,
    max_scratch_bytes: 40_000_000_000,
    max_cpu_millis: 7_200_000,
    max_gpu_millis: 7_200_000,
    max_output_bytes: 20_000_000_000,
    max_cost_microunits: 100_000_000,
    network: NetworkPolicy::Denied,
};

const EXTERNAL_ADAPTER: MediaSandboxLimits = MediaSandboxLimits {
    max_source_bytes: 2_000_000_000,
    max_duration_ms: 43_200_000,
    max_width: 7_680,
    max_height: 4_320,
    max_decoded_bytes: 32_000_000_000,
    max_frames: 4_000_000,
    max_tracks: 64,
    max_memory_bytes: 1_073_741_824,
    max_scratch_bytes: 2_000_000_000,
    max_cpu_millis: 900_000,
    max_gpu_millis: 0,
    max_output_bytes: 256_000_000,
    max_cost_microunits: 100_000_000,
    network: NetworkPolicy::DeclaredProviderAdapterOnly,
};

const HYBRID_RETRYABLE: &[ExecutionFailureClass] = &[
    ExecutionFailureClass::Quota,
    ExecutionFailureClass::Timeout,
    ExecutionFailureClass::ProviderOutage,
    ExecutionFailureClass::OutputIncompatible,
    ExecutionFailureClass::BetaRegression,
];
const NATIVE_RETRYABLE: &[ExecutionFailureClass] = &[
    ExecutionFailureClass::Timeout,
    ExecutionFailureClass::ProviderOutage,
];
const PROVIDER_RETRYABLE: &[ExecutionFailureClass] = &[
    ExecutionFailureClass::Quota,
    ExecutionFailureClass::Timeout,
    ExecutionFailureClass::ProviderOutage,
];

const MP4: &[&str] = &["video/mp4"];
const JPEG_PNG: &[&str] = &["image/jpeg", "image/png"];
const JPEG: &[&str] = &["image/jpeg"];
const M4A: &[&str] = &["audio/mp4"];
const JSON: &[&str] = &["application/json"];
const GIF_MP4: &[&str] = &["image/gif", "video/mp4"];
const AUDIO: &[&str] = &["audio/mpeg", "audio/mp4", "audio/wav"];
const VTT: &[&str] = &["text/vtt"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaServiceJobSpec {
    pub kind: MediaJobKind,
    pub input_role: MediaInputRole,
    pub output_role: MediaOutputRole,
    pub profile_id: &'static str,
    pub disposition: MediaExecutionDisposition,
    pub preferred: MediaExecutorKind,
    pub fallback: Option<MediaExecutorKind>,
    pub progress: ProgressCapability,
    pub cancellation: CancellationCapability,
    pub timeout_ms: u64,
    pub max_attempts: u16,
    pub retryable_failures: &'static [ExecutionFailureClass],
    pub idempotency: IdempotencyPolicy,
    pub output_content_types: &'static [&'static str],
    pub sandbox: MediaSandboxLimits,
    pub cap_sources: &'static [&'static str],
}

macro_rules! service_job {
    (
        $kind:ident, $input:ident, $output:ident, $profile:literal,
        $disposition:ident, $preferred:ident, $fallback:expr,
        $progress:ident, $cancel:ident, $timeout:expr, $attempts:expr,
        $retryable:expr, $idempotency:ident, $types:expr, $sandbox:expr,
        [$($source:literal),+ $(,)?]
    ) => {
        MediaServiceJobSpec {
            kind: MediaJobKind::$kind,
            input_role: MediaInputRole::$input,
            output_role: MediaOutputRole::$output,
            profile_id: $profile,
            disposition: MediaExecutionDisposition::$disposition,
            preferred: MediaExecutorKind::$preferred,
            fallback: $fallback,
            progress: ProgressCapability::$progress,
            cancellation: CancellationCapability::$cancel,
            timeout_ms: $timeout,
            max_attempts: $attempts,
            retryable_failures: $retryable,
            idempotency: IdempotencyPolicy::$idempotency,
            output_content_types: $types,
            sandbox: $sandbox,
            cap_sources: &[$($source),+],
        }
    };
}

const SERVICE_JOBS: &[MediaServiceJobSpec] = &[
    service_job!(
        OptimizedClip,
        SourceOriginal,
        Preview,
        "optimized_clip_v1",
        HybridManagedNative,
        CloudflareMedia,
        Some(MediaExecutorKind::NativeGstreamer),
        Indeterminate,
        SuppressPublication,
        120_000,
        3,
        HYBRID_RETRYABLE,
        DeterministicImmutableArtifact,
        MP4,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/process",
            "apps/media-server/src/lib/media-video.ts#processVideo"
        ]
    ),
    service_job!(
        Frame,
        SourceOriginal,
        Thumbnail,
        "thumbnail_v1",
        HybridManagedNative,
        CloudflareMedia,
        Some(MediaExecutorKind::NativeGstreamer),
        Indeterminate,
        SuppressPublication,
        60_000,
        3,
        HYBRID_RETRYABLE,
        DeterministicImmutableArtifact,
        JPEG_PNG,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/thumbnail",
            "apps/media-server/src/lib/media-video.ts#generateThumbnail"
        ]
    ),
    service_job!(
        Spritesheet,
        SourceOriginal,
        Spritesheet,
        "spritesheet_v1",
        HybridManagedNative,
        CloudflareMedia,
        Some(MediaExecutorKind::NativeGstreamer),
        Indeterminate,
        SuppressPublication,
        120_000,
        3,
        HYBRID_RETRYABLE,
        DeterministicImmutableArtifact,
        JPEG,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/lib/media-video.ts#generatePreviewGif",
            "target:replace-animated-preview-sampling"
        ]
    ),
    service_job!(
        AudioExtract,
        SourceOriginal,
        ExtractedAudio,
        "audio_extract_v1",
        HybridManagedNative,
        CloudflareMedia,
        Some(MediaExecutorKind::NativeGstreamer),
        Indeterminate,
        SuppressPublication,
        120_000,
        3,
        HYBRID_RETRYABLE,
        DeterministicImmutableArtifact,
        M4A,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/routes/audio.ts#/extract",
            "apps/media-server/src/lib/media-audio.ts#extractAudio"
        ]
    ),
    service_job!(
        Probe,
        SourceOriginal,
        ProbeManifest,
        "probe_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Indeterminate,
        InFlight,
        30_000,
        2,
        NATIVE_RETRYABLE,
        DeterministicManifest,
        JSON,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/probe",
            "apps/media-server/src/lib/media-probe.ts#probeVideo"
        ]
    ),
    service_job!(
        AudioPresence,
        SourceOriginal,
        ProbeManifest,
        "audio_presence_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Indeterminate,
        InFlight,
        30_000,
        2,
        NATIVE_RETRYABLE,
        DeterministicManifest,
        JSON,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/routes/audio.ts#/check",
            "apps/media-server/src/lib/media-audio.ts#checkHasAudioTrack"
        ]
    ),
    service_job!(
        DistributionMaster,
        SourceOriginal,
        DistributionMaster,
        "distribution_master_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        3_600_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        MP4,
        HEAVY_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/process",
            "apps/media-server/src/lib/media-video.ts#processVideo"
        ]
    ),
    service_job!(
        AnimatedPreview,
        DistributionMaster,
        AnimatedPreview,
        "animated_preview_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        300_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        GIF_MP4,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/lib/media-video.ts#generatePreviewGif",
            "apps/web/app/api/video/preview/route.ts"
        ]
    ),
    service_job!(
        AudioNormalize,
        ExtractedAudio,
        NormalizedAudio,
        "audio_normalize_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        900_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        AUDIO,
        LIGHT_NATIVE,
        [
            "apps/media-server/src/routes/audio.ts#/convert",
            "apps/media-server/src/lib/media-audio.ts#extractAudioStream"
        ]
    ),
    service_job!(
        RemuxRepair,
        SourceOriginal,
        RepairedMedia,
        "remux_repair_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        900_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        MP4,
        HEAVY_NATIVE,
        ["apps/media-server/src/lib/media-video.ts#repairContainer"]
    ),
    service_job!(
        SegmentMux,
        RecordingSegments,
        MuxedMedia,
        "segment_mux_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        1_800_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        MP4,
        HEAVY_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/mux-segments",
            "apps/media-server/src/lib/media-video.ts#muxMediaTracksToMp4"
        ]
    ),
    service_job!(
        Waveform,
        ExtractedAudio,
        Waveform,
        "waveform_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        900_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        JSON,
        LIGHT_NATIVE,
        ["target:retained-editor-waveform"]
    ),
    service_job!(
        Composition,
        EditTimeline,
        Composition,
        "composition_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        3_600_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        MP4,
        HEAVY_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/edit",
            "apps/media-server/src/lib/media-edit.ts#renderEditedVideo"
        ]
    ),
    service_job!(
        Normalize,
        SourceOriginal,
        NormalizedMedia,
        "normalize_v1",
        NativeOnly,
        NativeGstreamer,
        None,
        Monotonic,
        InFlight,
        900_000,
        3,
        NATIVE_RETRYABLE,
        DeterministicImmutableArtifact,
        MP4,
        HEAVY_NATIVE,
        [
            "apps/media-server/src/routes/video.ts#/convert",
            "apps/media-server/src/lib/media-video.ts#processVideo"
        ]
    ),
    service_job!(
        Transcription,
        ExtractedAudio,
        Captions,
        "transcription_v1",
        ExternalProviderAdapter,
        ExternalProvider,
        None,
        Indeterminate,
        SuppressPublication,
        3_600_000,
        4,
        PROVIDER_RETRYABLE,
        ProviderRequestAndResultDigest,
        VTT,
        EXTERNAL_ADAPTER,
        ["apps/web/workflows/transcribe.ts#transcribeWithDeepgram"]
    ),
    service_job!(
        AiCleanup,
        Captions,
        AiMetadata,
        "ai_cleanup_v1",
        ExternalProviderAdapter,
        ExternalProvider,
        None,
        Indeterminate,
        SuppressPublication,
        3_600_000,
        4,
        PROVIDER_RETRYABLE,
        ProviderRequestAndResultDigest,
        JSON,
        EXTERNAL_ADAPTER,
        ["apps/web/workflows/transcribe.ts#queueAiGeneration"]
    ),
];

#[derive(Debug, Clone, Copy)]
pub struct MediaServiceCatalog {
    pub version: u16,
    pub cap_reference_commit: &'static str,
    pub jobs: &'static [MediaServiceJobSpec],
}

#[must_use]
pub const fn media_service_catalog() -> MediaServiceCatalog {
    MediaServiceCatalog {
        version: MEDIA_SERVICE_CATALOG_VERSION,
        cap_reference_commit: CAP_MEDIA_REFERENCE_COMMIT,
        jobs: SERVICE_JOBS,
    }
}

impl MediaServiceCatalog {
    #[must_use]
    pub fn get(self, kind: MediaJobKind) -> Option<&'static MediaServiceJobSpec> {
        self.jobs.iter().find(|job| job.kind == kind)
    }

    pub fn validate(self) -> Result<Self, MediaServiceError> {
        if self.version != MEDIA_SERVICE_CATALOG_VERSION
            || self.cap_reference_commit != CAP_MEDIA_REFERENCE_COMMIT
            || self.jobs.len() != MediaJobKind::ALL.len()
            || media_job_catalog().jobs.len() != self.jobs.len()
        {
            return Err(MediaServiceError::InvalidCatalog);
        }
        for kind in MediaJobKind::ALL {
            let matching: Vec<_> = self.jobs.iter().filter(|job| job.kind == kind).collect();
            if matching.len() != 1 {
                return Err(MediaServiceError::InvalidCatalog);
            }
            let job = matching[0];
            if !safe_label(job.profile_id)
                || job.timeout_ms == 0
                || job.max_attempts == 0
                || job.retryable_failures.is_empty()
                || job.output_content_types.is_empty()
                || job.cap_sources.is_empty()
                || job.sandbox.max_source_bytes == 0
                || job.sandbox.max_duration_ms == 0
                || job.sandbox.max_output_bytes == 0
            {
                return Err(MediaServiceError::InvalidCatalog);
            }
            match job.disposition {
                MediaExecutionDisposition::HybridManagedNative
                    if job.preferred != MediaExecutorKind::CloudflareMedia
                        || job.fallback != Some(MediaExecutorKind::NativeGstreamer) =>
                {
                    return Err(MediaServiceError::InvalidCatalog);
                }
                MediaExecutionDisposition::NativeOnly
                    if job.preferred != MediaExecutorKind::NativeGstreamer
                        || job.fallback.is_some() =>
                {
                    return Err(MediaServiceError::InvalidCatalog);
                }
                MediaExecutionDisposition::ExternalProviderAdapter
                    if job.preferred != MediaExecutorKind::ExternalProvider
                        || job.fallback.is_some() =>
                {
                    return Err(MediaServiceError::InvalidCatalog);
                }
                _ => {}
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedOutputMode {
    Video,
    Frame,
    Spritesheet,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeFit {
    Contain,
    Cover,
    ScaleDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOutputFormat {
    Mp4H264Aac,
    Jpeg,
    Png,
    M4aAac,
}

impl MediaOutputFormat {
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Mp4H264Aac => "video/mp4",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::M4aAac => "audio/mp4",
        }
    }

    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4H264Aac => "mp4",
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::M4aAac => "m4a",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTransformProfile {
    pub schema_version: u16,
    pub profile_id: String,
    pub profile_version: u16,
    pub mode: ManagedOutputMode,
    pub start_ms: u64,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: ResizeFit,
    pub image_count: Option<u16>,
    pub include_audio: bool,
    pub format: MediaOutputFormat,
    pub max_output_bytes: u64,
}

impl MediaTransformProfile {
    pub fn validate_for(&self, kind: MediaJobKind) -> Result<(), MediaServiceError> {
        let spec = media_service_catalog()
            .get(kind)
            .ok_or(MediaServiceError::UnknownJob)?;
        if self.schema_version != MEDIA_TRANSFORM_PROFILE_SCHEMA_VERSION
            || self.profile_version == 0
            || self.profile_id != spec.profile_id
            || !safe_label(&self.profile_id)
            || self.max_output_bytes == 0
            || self.max_output_bytes > spec.sandbox.max_output_bytes
            || self.start_ms > 600_000
        {
            return Err(MediaServiceError::InvalidProfile);
        }
        if self.width.is_some() != self.height.is_some() {
            return Err(MediaServiceError::InvalidProfile);
        }
        let valid = match kind {
            MediaJobKind::OptimizedClip => {
                self.mode == ManagedOutputMode::Video
                    && self.format == MediaOutputFormat::Mp4H264Aac
                    && self.duration_ms.is_some()
                    && self.image_count.is_none()
            }
            MediaJobKind::Frame => {
                self.mode == ManagedOutputMode::Frame
                    && matches!(
                        self.format,
                        MediaOutputFormat::Jpeg | MediaOutputFormat::Png
                    )
                    && self.duration_ms.is_none()
                    && self.image_count.is_none()
                    && !self.include_audio
            }
            MediaJobKind::Spritesheet => {
                self.mode == ManagedOutputMode::Spritesheet
                    && self.format == MediaOutputFormat::Jpeg
                    && self.duration_ms.is_some()
                    && self.image_count.is_some_and(|count| count > 0)
                    && !self.include_audio
            }
            MediaJobKind::AudioExtract => {
                self.mode == ManagedOutputMode::Audio
                    && self.format == MediaOutputFormat::M4aAac
                    && self.duration_ms.is_some()
                    && self.image_count.is_none()
                    && self.width.is_none()
                    && !self.include_audio
            }
            _ => false,
        };
        if !valid {
            return Err(MediaServiceError::InvalidProfile);
        }
        Ok(())
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        format!(
            "{}\0{}\0{}\0{:?}\0{}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{}\0{:?}\0{}",
            self.schema_version,
            self.profile_id,
            self.profile_version,
            self.mode,
            self.start_ms,
            self.duration_ms,
            self.width,
            self.height,
            self.fit,
            self.image_count,
            self.include_audio,
            self.format,
            self.max_output_bytes
        )
        .into_bytes()
    }

    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes()).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeTrust {
    VerifiedUploadManifest,
    VerifiedNativeProbe,
    UntrustedCaller,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PrivateMediaInput {
    pub tenant_id: String,
    pub video_id: String,
    pub object_key: String,
    pub role: MediaInputRole,
    pub source_version: u32,
    pub source_sha256: String,
    pub metadata: MediaInput,
    pub decoded_bytes_upper_bound: u64,
    pub frame_count_upper_bound: u64,
    pub track_count: u16,
    pub probe_trust: ProbeTrust,
}

impl fmt::Debug for PrivateMediaInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateMediaInput")
            .field("tenant_id", &"[redacted]")
            .field("video_id", &"[redacted]")
            .field("object_key", &"[redacted]")
            .field("role", &self.role)
            .field("source_version", &self.source_version)
            .field("source_sha256", &"[redacted]")
            .field("metadata", &self.metadata)
            .field("decoded_bytes_upper_bound", &self.decoded_bytes_upper_bound)
            .field("frame_count_upper_bound", &self.frame_count_upper_bound)
            .field("track_count", &self.track_count)
            .field("probe_trust", &self.probe_trust)
            .finish()
    }
}

impl PrivateMediaInput {
    pub fn validate(&self) -> Result<(), MediaServiceError> {
        self.metadata
            .validate()
            .map_err(|_| MediaServiceError::InvalidInput)?;
        if !canonical_uuid(&self.tenant_id)
            || !canonical_uuid(&self.video_id)
            || self.source_version == 0
            || self.source_sha256.len() != 64
            || !self
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || self.decoded_bytes_upper_bound == 0
            || self.frame_count_upper_bound == 0
            || self.track_count == 0
            || self.object_key.contains("://")
            || self.object_key.contains(['?', '#', '\\'])
            || self.object_key.len() > 1_024
            || self
                .object_key
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
            || !self.object_key.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.')
            })
        {
            return Err(MediaServiceError::InvalidInput);
        }
        let required_prefix = format!("tenants/{}/videos/{}/", self.tenant_id, self.video_id);
        if !self.object_key.starts_with(&required_prefix)
            || self.probe_trust == ProbeTrust::UntrustedCaller
        {
            return Err(MediaServiceError::UntrustedInput);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedMediaContract {
    pub revision: String,
    pub documentation_date: String,
    pub max_input_bytes_exclusive: u64,
    pub max_input_duration_ms: u64,
    pub max_input_width: u32,
    pub max_input_height: u32,
    pub max_start_ms: u64,
    pub min_output_duration_ms: u64,
    pub max_output_duration_ms: u64,
    pub min_output_dimension: u32,
    pub max_output_dimension: u32,
    pub max_image_count: u16,
    pub max_video_output_bytes: u64,
    pub max_frame_output_bytes: u64,
    pub max_spritesheet_output_bytes: u64,
    pub max_audio_output_bytes: u64,
}

impl ManagedMediaContract {
    #[must_use]
    pub fn cloudflare_2026_06_10() -> Self {
        Self {
            revision: "cloudflare-media-binding-2026-06-10".into(),
            documentation_date: "2026-06-10".into(),
            max_input_bytes_exclusive: 100_000_000,
            max_input_duration_ms: 600_000,
            // Vendor documentation does not publish an input-resolution maximum;
            // this operator ceiling is deliberately lower than the native envelope
            // and must be exercised by the remote contract lane.
            max_input_width: 7_680,
            max_input_height: 4_320,
            max_start_ms: 600_000,
            min_output_duration_ms: 1_000,
            max_output_duration_ms: 60_000,
            min_output_dimension: 10,
            max_output_dimension: 2_000,
            max_image_count: 100,
            // The current Worker adapter incrementally consumes the binding
            // stream but must buffer once to calculate the R2 SHA-256 before a
            // conditional immutable PUT. Keep every managed result below this
            // explicit operator memory envelope.
            max_video_output_bytes: 32_000_000,
            max_frame_output_bytes: 32_000_000,
            max_spritesheet_output_bytes: 32_000_000,
            max_audio_output_bytes: 32_000_000,
        }
    }

    pub fn validate(&self) -> Result<(), MediaServiceError> {
        if !safe_label(&self.revision)
            || self.documentation_date.len() != 10
            || self.max_input_bytes_exclusive <= 1
            || self.max_input_duration_ms == 0
            || self.max_input_width == 0
            || self.max_input_height == 0
            || self.min_output_duration_ms == 0
            || self.min_output_duration_ms > self.max_output_duration_ms
            || self.min_output_dimension == 0
            || self.min_output_dimension > self.max_output_dimension
            || self.max_image_count == 0
            || self.max_video_output_bytes == 0
            || self.max_frame_output_bytes == 0
            || self.max_spritesheet_output_bytes == 0
            || self.max_audio_output_bytes == 0
        {
            return Err(MediaServiceError::InvalidLimits);
        }
        Ok(())
    }

    const fn max_output_bytes(&self, mode: ManagedOutputMode) -> u64 {
        match mode {
            ManagedOutputMode::Video => self.max_video_output_bytes,
            ManagedOutputMode::Frame => self.max_frame_output_bytes,
            ManagedOutputMode::Spritesheet => self.max_spritesheet_output_bytes,
            ManagedOutputMode::Audio => self.max_audio_output_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeMediaContract {
    pub limits: MediaSandboxLimits,
    pub max_decompression_ratio: u64,
}

impl NativeMediaContract {
    pub fn validate(self) -> Result<Self, MediaServiceError> {
        if self.limits.max_source_bytes == 0
            || self.limits.max_duration_ms == 0
            || self.limits.max_width == 0
            || self.limits.max_height == 0
            || self.limits.max_decoded_bytes == 0
            || self.limits.max_frames == 0
            || self.limits.max_tracks == 0
            || self.limits.max_memory_bytes == 0
            || self.limits.max_scratch_bytes == 0
            || self.limits.max_cpu_millis == 0
            || self.limits.max_output_bytes == 0
            || self.max_decompression_ratio == 0
        {
            return Err(MediaServiceError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaServiceRouteReason {
    ManagedPreferred,
    ManagedKillSwitch,
    ManagedInputFormat,
    ManagedInputLimit,
    ManagedProfileLimit,
    NativeOnly,
    ExternalProvider,
    ManagedFailure(ExecutionFailureClass),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaServiceRouteDecision {
    pub catalog_version: u16,
    pub kind: MediaJobKind,
    pub executor: MediaExecutorKind,
    pub reason: MediaServiceRouteReason,
    pub fallback_available: bool,
    pub timeout_ms: u64,
    pub progress: ProgressCapability,
    pub cancellation: CancellationCapability,
}

#[derive(Debug, Clone)]
pub struct MediaCapabilityRouter {
    managed_enabled: bool,
    managed: ManagedMediaContract,
    native: NativeMediaContract,
}

impl MediaCapabilityRouter {
    pub fn new(
        managed_enabled: bool,
        managed: ManagedMediaContract,
        native: NativeMediaContract,
    ) -> Result<Self, MediaServiceError> {
        managed.validate()?;
        Ok(Self {
            managed_enabled,
            managed,
            native: native.validate()?,
        })
    }

    pub fn route(
        &self,
        kind: MediaJobKind,
        input: &PrivateMediaInput,
        profile: Option<&MediaTransformProfile>,
    ) -> Result<MediaServiceRouteDecision, MediaServiceError> {
        input.validate()?;
        let spec = media_service_catalog()
            .validate()?
            .get(kind)
            .ok_or(MediaServiceError::UnknownJob)?;
        if input.role != spec.input_role {
            return Err(MediaServiceError::InvalidInputRole);
        }
        self.ensure_parser_safety(spec, input)?;
        match spec.disposition {
            MediaExecutionDisposition::ExternalProviderAdapter => Ok(service_decision(
                spec,
                MediaExecutorKind::ExternalProvider,
                MediaServiceRouteReason::ExternalProvider,
                false,
            )),
            MediaExecutionDisposition::NativeOnly => {
                if profile.is_some() {
                    return Err(MediaServiceError::InvalidProfile);
                }
                self.ensure_native(spec, input, None)?;
                Ok(service_decision(
                    spec,
                    MediaExecutorKind::NativeGstreamer,
                    MediaServiceRouteReason::NativeOnly,
                    false,
                ))
            }
            MediaExecutionDisposition::HybridManagedNative => {
                let profile = profile.ok_or(MediaServiceError::InvalidProfile)?;
                profile.validate_for(kind)?;
                if !self.managed_enabled {
                    self.ensure_native(spec, input, Some(profile))?;
                    return Ok(service_decision(
                        spec,
                        MediaExecutorKind::NativeGstreamer,
                        MediaServiceRouteReason::ManagedKillSwitch,
                        false,
                    ));
                }
                if let Some(reason) = self.managed_rejection(input, profile)? {
                    self.ensure_native(spec, input, Some(profile))?;
                    return Ok(service_decision(
                        spec,
                        MediaExecutorKind::NativeGstreamer,
                        reason,
                        false,
                    ));
                }
                let fallback_available = self.ensure_native(spec, input, Some(profile)).is_ok();
                Ok(service_decision(
                    spec,
                    MediaExecutorKind::CloudflareMedia,
                    MediaServiceRouteReason::ManagedPreferred,
                    fallback_available,
                ))
            }
        }
    }

    pub fn fallback_after_failure(
        &self,
        current: MediaServiceRouteDecision,
        failure: ExecutionFailureClass,
        input: &PrivateMediaInput,
        profile: &MediaTransformProfile,
    ) -> Result<MediaServiceRouteDecision, MediaServiceError> {
        if current.executor != MediaExecutorKind::CloudflareMedia
            || !current.fallback_available
            || matches!(
                failure,
                ExecutionFailureClass::InvalidInput
                    | ExecutionFailureClass::SecurityViolation
                    | ExecutionFailureClass::Cancelled
            )
        {
            return Err(MediaServiceError::FallbackForbidden);
        }
        let spec = media_service_catalog()
            .get(current.kind)
            .ok_or(MediaServiceError::UnknownJob)?;
        if !spec.retryable_failures.contains(&failure) {
            return Err(MediaServiceError::FallbackForbidden);
        }
        profile.validate_for(current.kind)?;
        self.ensure_native(spec, input, Some(profile))?;
        Ok(service_decision(
            spec,
            MediaExecutorKind::NativeGstreamer,
            MediaServiceRouteReason::ManagedFailure(failure),
            false,
        ))
    }

    fn managed_rejection(
        &self,
        input: &PrivateMediaInput,
        profile: &MediaTransformProfile,
    ) -> Result<Option<MediaServiceRouteReason>, MediaServiceError> {
        self.managed.validate()?;
        if input.metadata.bytes >= self.managed.max_input_bytes_exclusive
            || input.metadata.duration_ms > self.managed.max_input_duration_ms
            || input.metadata.width > self.managed.max_input_width
            || input.metadata.height > self.managed.max_input_height
        {
            return Ok(Some(MediaServiceRouteReason::ManagedInputLimit));
        }
        if input.metadata.container != ContainerFormat::Mp4
            || input.metadata.video_codec != VideoCodec::H264
            || !matches!(
                input.metadata.audio_codec,
                AudioCodec::Aac | AudioCodec::Mp3 | AudioCodec::None
            )
        {
            return Ok(Some(MediaServiceRouteReason::ManagedInputFormat));
        }
        if profile.start_ms > self.managed.max_start_ms
            || profile.start_ms >= input.metadata.duration_ms
            || !profile.start_ms.is_multiple_of(1_000)
            || profile.duration_ms.is_some_and(|duration| {
                duration < self.managed.min_output_duration_ms
                    || duration > self.managed.max_output_duration_ms
                    || !duration.is_multiple_of(1_000)
                    || profile.start_ms.saturating_add(duration) > input.metadata.duration_ms
            })
            || profile.width.is_some_and(|width| {
                width < self.managed.min_output_dimension
                    || width > self.managed.max_output_dimension
            })
            || profile.height.is_some_and(|height| {
                height < self.managed.min_output_dimension
                    || height > self.managed.max_output_dimension
            })
            || profile
                .image_count
                .is_some_and(|count| count > self.managed.max_image_count)
            || profile.max_output_bytes > self.managed.max_output_bytes(profile.mode)
        {
            return Ok(Some(MediaServiceRouteReason::ManagedProfileLimit));
        }
        Ok(None)
    }

    fn ensure_native(
        &self,
        spec: &MediaServiceJobSpec,
        input: &PrivateMediaInput,
        profile: Option<&MediaTransformProfile>,
    ) -> Result<(), MediaServiceError> {
        let limits = self.native.limits;
        let pixels = u64::from(input.metadata.width)
            .checked_mul(u64::from(input.metadata.height))
            .ok_or(MediaServiceError::ResourceLimit)?;
        if input.metadata.bytes > limits.max_source_bytes.min(spec.sandbox.max_source_bytes)
            || input.metadata.duration_ms > limits.max_duration_ms.min(spec.sandbox.max_duration_ms)
            || input.metadata.width > limits.max_width.min(spec.sandbox.max_width)
            || input.metadata.height > limits.max_height.min(spec.sandbox.max_height)
            || pixels
                > u64::from(limits.max_width.min(spec.sandbox.max_width))
                    .saturating_mul(u64::from(limits.max_height.min(spec.sandbox.max_height)))
            || input.decoded_bytes_upper_bound
                > limits.max_decoded_bytes.min(spec.sandbox.max_decoded_bytes)
            || input.frame_count_upper_bound > limits.max_frames.min(spec.sandbox.max_frames)
            || input.track_count > limits.max_tracks.min(spec.sandbox.max_tracks)
            || input.decoded_bytes_upper_bound
                > input
                    .metadata
                    .bytes
                    .saturating_mul(self.native.max_decompression_ratio)
            || profile.is_some_and(|profile| {
                profile.max_output_bytes
                    > limits.max_output_bytes.min(spec.sandbox.max_output_bytes)
            })
        {
            return Err(MediaServiceError::ResourceLimit);
        }
        Ok(())
    }

    fn ensure_parser_safety(
        &self,
        spec: &MediaServiceJobSpec,
        input: &PrivateMediaInput,
    ) -> Result<(), MediaServiceError> {
        if input.decoded_bytes_upper_bound > spec.sandbox.max_decoded_bytes
            || input.frame_count_upper_bound > spec.sandbox.max_frames
            || input.track_count > spec.sandbox.max_tracks
            || input.metadata.width > spec.sandbox.max_width
            || input.metadata.height > spec.sandbox.max_height
            || input.decoded_bytes_upper_bound
                > input
                    .metadata
                    .bytes
                    .saturating_mul(self.native.max_decompression_ratio)
        {
            return Err(MediaServiceError::ResourceLimit);
        }
        Ok(())
    }
}

fn service_decision(
    spec: &MediaServiceJobSpec,
    executor: MediaExecutorKind,
    reason: MediaServiceRouteReason,
    fallback_available: bool,
) -> MediaServiceRouteDecision {
    MediaServiceRouteDecision {
        catalog_version: MEDIA_SERVICE_CATALOG_VERSION,
        kind: spec.kind,
        executor,
        reason,
        fallback_available,
        timeout_ms: spec.timeout_ms,
        progress: spec.progress,
        cancellation: spec.cancellation,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MediaArtifactIdentity {
    pub output_key: String,
    pub manifest_key: String,
    pub source_sha256: String,
    pub profile_sha256: String,
    pub content_type: String,
}

impl fmt::Debug for MediaArtifactIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaArtifactIdentity")
            .field("output_key", &"[redacted]")
            .field("manifest_key", &"[redacted]")
            .field("source_sha256", &"[redacted]")
            .field("profile_sha256", &"[redacted]")
            .field("content_type", &self.content_type)
            .finish()
    }
}

impl MediaArtifactIdentity {
    pub fn derive(
        input: &PrivateMediaInput,
        profile: &MediaTransformProfile,
        kind: MediaJobKind,
    ) -> Result<Self, MediaServiceError> {
        input.validate()?;
        profile.validate_for(kind)?;
        let profile_sha256 = hex(&profile.sha256());
        Self::derive_declared(
            input,
            kind,
            profile.profile_version,
            &profile_sha256,
            profile.format.content_type(),
            profile.format.extension(),
        )
    }

    pub fn derive_declared(
        input: &PrivateMediaInput,
        kind: MediaJobKind,
        profile_version: u16,
        normalized_profile_sha256: &str,
        content_type: &str,
        extension: &str,
    ) -> Result<Self, MediaServiceError> {
        input.validate()?;
        let spec = media_service_catalog()
            .get(kind)
            .ok_or(MediaServiceError::UnknownJob)?;
        if input.role != spec.input_role
            || profile_version == 0
            || !valid_sha256(normalized_profile_sha256)
            || !spec.output_content_types.contains(&content_type)
            || extension.is_empty()
            || extension.len() > 8
            || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(MediaServiceError::InvalidProfile);
        }
        let mut logical = Sha256::new();
        logical.update(b"frame-media-artifact-v1\0");
        logical.update(input.source_sha256.as_bytes());
        logical.update([0]);
        logical.update(kind.id().as_bytes());
        logical.update([0]);
        logical.update(normalized_profile_sha256.as_bytes());
        let logical_digest = hex(&logical.finalize());
        let base = format!(
            "tenants/{}/videos/{}/derivatives/{}/v{}/{}",
            input.tenant_id, input.video_id, spec.profile_id, input.source_version, logical_digest
        );
        Ok(Self {
            output_key: format!("{base}.{extension}"),
            manifest_key: format!("{base}.manifest.json"),
            source_sha256: input.source_sha256.clone(),
            profile_sha256: normalized_profile_sha256.into(),
            content_type: content_type.into(),
        })
    }

    pub fn staging_key(&self, attempt: u16) -> Result<String, MediaServiceError> {
        if attempt == 0 {
            return Err(MediaServiceError::InvalidAttempt);
        }
        Ok(format!("{}.attempt-{attempt}.partial", self.output_key))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaExecutionCost {
    pub provider_operations: u64,
    pub provider_output_seconds: u64,
    pub native_cpu_millis: u64,
    pub native_gpu_millis: u64,
    pub scratch_byte_seconds: u64,
    pub total_microunits: u64,
}

impl MediaExecutionCost {
    fn checked_add(self, delta: Self) -> Result<Self, MediaServiceError> {
        Ok(Self {
            provider_operations: self
                .provider_operations
                .checked_add(delta.provider_operations)
                .ok_or(MediaServiceError::CostOverflow)?,
            provider_output_seconds: self
                .provider_output_seconds
                .checked_add(delta.provider_output_seconds)
                .ok_or(MediaServiceError::CostOverflow)?,
            native_cpu_millis: self
                .native_cpu_millis
                .checked_add(delta.native_cpu_millis)
                .ok_or(MediaServiceError::CostOverflow)?,
            native_gpu_millis: self
                .native_gpu_millis
                .checked_add(delta.native_gpu_millis)
                .ok_or(MediaServiceError::CostOverflow)?,
            scratch_byte_seconds: self
                .scratch_byte_seconds
                .checked_add(delta.scratch_byte_seconds)
                .ok_or(MediaServiceError::CostOverflow)?,
            total_microunits: self
                .total_microunits
                .checked_add(delta.total_microunits)
                .ok_or(MediaServiceError::CostOverflow)?,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MediaAttemptFence {
    pub attempt: u16,
    pub lease_epoch: u64,
    pub lease_expires_at_ms: u64,
    lease_token_sha256: [u8; 32],
}

impl fmt::Debug for MediaAttemptFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaAttemptFence")
            .field("attempt", &self.attempt)
            .field("lease_epoch", &self.lease_epoch)
            .field("lease_expires_at_ms", &self.lease_expires_at_ms)
            .field("lease_token_sha256", &"[redacted]")
            .finish()
    }
}

impl MediaAttemptFence {
    pub fn new(
        attempt: u16,
        lease_epoch: u64,
        lease_expires_at_ms: u64,
        lease_token: &[u8],
    ) -> Result<Self, MediaServiceError> {
        if attempt == 0
            || lease_epoch == 0
            || lease_expires_at_ms == 0
            || !(32..=512).contains(&lease_token.len())
        {
            return Err(MediaServiceError::InvalidAttempt);
        }
        Ok(Self {
            attempt,
            lease_epoch,
            lease_expires_at_ms,
            lease_token_sha256: Sha256::digest(lease_token).into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableMediaJobState {
    Queued,
    Running,
    Publishing,
    RetryableFailure,
    RecoveryRequired,
    CancelRequested,
    Ready,
    Failed,
    Cancelled,
}

impl DurableMediaJobState {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Ready | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StagedMediaOutput {
    pub staging_key: String,
    pub final_output_key: String,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
    pub source_sha256: String,
    pub profile_sha256: String,
    pub verified_at_ms: u64,
}

impl fmt::Debug for StagedMediaOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedMediaOutput")
            .field("staging_key", &"[redacted]")
            .field("final_output_key", &"[redacted]")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"[redacted]")
            .field("content_type", &self.content_type)
            .field("source_sha256", &"[redacted]")
            .field("profile_sha256", &"[redacted]")
            .field("verified_at_ms", &self.verified_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaOutputVerification {
    pub playable: bool,
    pub metadata_passed: bool,
    pub perceptual_passed: bool,
    pub waveform_passed: bool,
    pub caption_timing_passed: bool,
}

impl MediaOutputVerification {
    #[must_use]
    pub const fn all_passed(self) -> bool {
        self.playable
            && self.metadata_passed
            && self.perceptual_passed
            && self.waveform_passed
            && self.caption_timing_passed
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MediaOutputManifest {
    pub schema_version: u16,
    pub output_key: String,
    pub manifest_key: String,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
    pub source_sha256: String,
    pub profile_sha256: String,
    pub executor: MediaExecutorKind,
    pub attempt: u16,
    pub committed_at_ms: u64,
    pub verification: MediaOutputVerification,
    pub cost: MediaExecutionCost,
}

impl fmt::Debug for MediaOutputManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaOutputManifest")
            .field("schema_version", &self.schema_version)
            .field("output_key", &"[redacted]")
            .field("manifest_key", &"[redacted]")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"[redacted]")
            .field("content_type", &self.content_type)
            .field("source_sha256", &"[redacted]")
            .field("profile_sha256", &"[redacted]")
            .field("executor", &self.executor)
            .field("attempt", &self.attempt)
            .field("committed_at_ms", &self.committed_at_ms)
            .field("verification", &self.verification)
            .field("cost", &self.cost)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalMutation {
    Applied,
    Replayed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DurableMediaJob {
    pub schema_version: u16,
    job_id: String,
    pub kind: MediaJobKind,
    pub artifact: MediaArtifactIdentity,
    pub state: DurableMediaJobState,
    pub cancel_requested_at_ms: Option<u64>,
    pub next_attempt: u16,
    pub active_executor: Option<MediaExecutorKind>,
    pub active_fence: Option<MediaAttemptFence>,
    pub progress_basis_points: Option<u16>,
    pub staged: Option<StagedMediaOutput>,
    pub manifest: Option<MediaOutputManifest>,
    pub accumulated_cost: MediaExecutionCost,
    pub last_failure: Option<ExecutionFailureClass>,
}

impl fmt::Debug for DurableMediaJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableMediaJob")
            .field("schema_version", &self.schema_version)
            .field("job_id", &"[redacted]")
            .field("kind", &self.kind)
            .field("artifact", &self.artifact)
            .field("state", &self.state)
            .field("cancel_requested_at_ms", &self.cancel_requested_at_ms)
            .field("next_attempt", &self.next_attempt)
            .field("active_executor", &self.active_executor)
            .field("active_fence", &self.active_fence)
            .field("progress_basis_points", &self.progress_basis_points)
            .field("has_staged_output", &self.staged.is_some())
            .field("has_manifest", &self.manifest.is_some())
            .field("accumulated_cost", &self.accumulated_cost)
            .field("last_failure", &self.last_failure)
            .finish()
    }
}

impl DurableMediaJob {
    pub fn new(
        job_id: impl Into<String>,
        kind: MediaJobKind,
        artifact: MediaArtifactIdentity,
    ) -> Result<Self, MediaServiceError> {
        media_service_catalog().validate()?;
        let job_id = job_id.into();
        if !canonical_uuid(&job_id) || media_service_catalog().get(kind).is_none() {
            return Err(MediaServiceError::InvalidJob);
        }
        Ok(Self {
            schema_version: MEDIA_JOB_JOURNAL_SCHEMA_VERSION,
            job_id,
            kind,
            artifact,
            state: DurableMediaJobState::Queued,
            cancel_requested_at_ms: None,
            next_attempt: 1,
            active_executor: None,
            active_fence: None,
            progress_basis_points: None,
            staged: None,
            manifest: None,
            accumulated_cost: MediaExecutionCost::default(),
            last_failure: None,
        })
    }

    pub fn claim(
        &mut self,
        executor: MediaExecutorKind,
        lease_epoch: u64,
        now_ms: u64,
        lease_expires_at_ms: u64,
        lease_token: &[u8],
    ) -> Result<MediaAttemptFence, MediaServiceError> {
        self.validate_persisted()?;
        let spec = media_service_catalog()
            .get(self.kind)
            .ok_or(MediaServiceError::UnknownJob)?;
        if !matches!(
            self.state,
            DurableMediaJobState::Queued | DurableMediaJobState::RetryableFailure
        ) || self.active_fence.is_some()
            || self.next_attempt > spec.max_attempts
            || lease_expires_at_ms <= now_ms
            || !executor_allowed(spec, executor)
        {
            return Err(MediaServiceError::InvalidTransition);
        }
        let fence = MediaAttemptFence::new(
            self.next_attempt,
            lease_epoch,
            lease_expires_at_ms,
            lease_token,
        )?;
        self.next_attempt = self
            .next_attempt
            .checked_add(1)
            .ok_or(MediaServiceError::InvalidAttempt)?;
        self.active_executor = Some(executor);
        self.active_fence = Some(fence);
        self.progress_basis_points = match spec.progress {
            ProgressCapability::Monotonic => Some(0),
            ProgressCapability::Indeterminate => None,
        };
        self.staged = None;
        self.state = DurableMediaJobState::Running;
        Ok(fence)
    }

    pub fn record_progress(
        &mut self,
        fence: MediaAttemptFence,
        now_ms: u64,
        basis_points: u16,
    ) -> Result<JournalMutation, MediaServiceError> {
        self.require_fence(fence, now_ms)?;
        if self.state != DurableMediaJobState::Running || basis_points >= 10_000 {
            return Err(MediaServiceError::InvalidTransition);
        }
        let current = self
            .progress_basis_points
            .ok_or(MediaServiceError::IndeterminateProgress)?;
        if basis_points < current {
            return Err(MediaServiceError::ProgressRegression);
        }
        if basis_points == current {
            return Ok(JournalMutation::Replayed);
        }
        self.progress_basis_points = Some(basis_points);
        Ok(JournalMutation::Applied)
    }

    pub fn charge(
        &mut self,
        fence: MediaAttemptFence,
        now_ms: u64,
        delta: MediaExecutionCost,
    ) -> Result<JournalMutation, MediaServiceError> {
        self.require_fence(fence, now_ms)?;
        if !matches!(
            self.state,
            DurableMediaJobState::Running | DurableMediaJobState::Publishing
        ) {
            return Err(MediaServiceError::InvalidTransition);
        }
        let next = self.accumulated_cost.checked_add(delta)?;
        let max = media_service_catalog()
            .get(self.kind)
            .ok_or(MediaServiceError::UnknownJob)?
            .sandbox
            .max_cost_microunits;
        if next.total_microunits > max {
            return Err(MediaServiceError::CostLimit);
        }
        self.accumulated_cost = next;
        Ok(JournalMutation::Applied)
    }

    pub fn record_staged(
        &mut self,
        fence: MediaAttemptFence,
        now_ms: u64,
        staged: StagedMediaOutput,
    ) -> Result<JournalMutation, MediaServiceError> {
        self.require_fence(fence, now_ms)?;
        if self.state != DurableMediaJobState::Running || self.cancel_requested_at_ms.is_some() {
            return Err(MediaServiceError::PublicationSuppressed);
        }
        self.validate_staged(fence, &staged)?;
        if self
            .staged
            .as_ref()
            .is_some_and(|existing| existing == &staged)
        {
            return Ok(JournalMutation::Replayed);
        }
        if self.staged.is_some() {
            return Err(MediaServiceError::PublicationConflict);
        }
        self.staged = Some(staged);
        self.state = DurableMediaJobState::Publishing;
        Ok(JournalMutation::Applied)
    }

    pub fn commit(
        &mut self,
        fence: MediaAttemptFence,
        now_ms: u64,
        manifest: MediaOutputManifest,
    ) -> Result<JournalMutation, MediaServiceError> {
        if self.state == DurableMediaJobState::Ready {
            return if self.manifest.as_ref() == Some(&manifest) {
                Ok(JournalMutation::Replayed)
            } else {
                Err(MediaServiceError::PublicationConflict)
            };
        }
        self.require_fence(fence, now_ms)?;
        if self.state != DurableMediaJobState::Publishing || self.cancel_requested_at_ms.is_some() {
            return Err(MediaServiceError::PublicationSuppressed);
        }
        let staged = self
            .staged
            .as_ref()
            .ok_or(MediaServiceError::UnverifiedOutput)?;
        if manifest.schema_version != MEDIA_JOB_JOURNAL_SCHEMA_VERSION
            || manifest.output_key != self.artifact.output_key
            || manifest.manifest_key != self.artifact.manifest_key
            || manifest.bytes != staged.bytes
            || manifest.checksum_sha256 != staged.checksum_sha256
            || manifest.content_type != staged.content_type
            || manifest.source_sha256 != self.artifact.source_sha256
            || manifest.profile_sha256 != self.artifact.profile_sha256
            || manifest.executor
                != self
                    .active_executor
                    .ok_or(MediaServiceError::InvalidAttempt)?
            || manifest.attempt != fence.attempt
            || manifest.committed_at_ms < staged.verified_at_ms
            || manifest.committed_at_ms > now_ms
            || manifest.cost != self.accumulated_cost
            || !manifest.verification.clone().all_passed()
        {
            return Err(MediaServiceError::UnverifiedOutput);
        }
        self.manifest = Some(manifest);
        self.state = DurableMediaJobState::Ready;
        self.progress_basis_points = Some(10_000);
        self.active_executor = None;
        self.active_fence = None;
        Ok(JournalMutation::Applied)
    }

    pub fn fail(
        &mut self,
        fence: MediaAttemptFence,
        now_ms: u64,
        failure: ExecutionFailureClass,
        publication_suppressed: bool,
    ) -> Result<JournalMutation, MediaServiceError> {
        self.require_fence(fence, now_ms)?;
        if self.state == DurableMediaJobState::Publishing && !publication_suppressed {
            self.state = DurableMediaJobState::RecoveryRequired;
            self.last_failure = Some(failure);
            return Ok(JournalMutation::Applied);
        }
        if self.cancel_requested_at_ms.is_some() || failure == ExecutionFailureClass::Cancelled {
            if !publication_suppressed {
                self.state = DurableMediaJobState::RecoveryRequired;
                return Ok(JournalMutation::Applied);
            }
            self.state = DurableMediaJobState::Cancelled;
        } else {
            let spec = media_service_catalog()
                .get(self.kind)
                .ok_or(MediaServiceError::UnknownJob)?;
            self.state = if spec.retryable_failures.contains(&failure)
                && fence.attempt < spec.max_attempts
            {
                DurableMediaJobState::RetryableFailure
            } else {
                DurableMediaJobState::Failed
            };
        }
        self.last_failure = Some(failure);
        self.active_executor = None;
        self.active_fence = None;
        self.staged = None;
        Ok(JournalMutation::Applied)
    }

    pub fn request_cancel(&mut self, now_ms: u64) -> Result<JournalMutation, MediaServiceError> {
        if self.state.terminal() {
            return if self.state == DurableMediaJobState::Cancelled {
                Ok(JournalMutation::Replayed)
            } else {
                Err(MediaServiceError::InvalidTransition)
            };
        }
        if self.cancel_requested_at_ms.is_some() {
            return Ok(JournalMutation::Replayed);
        }
        self.cancel_requested_at_ms = Some(now_ms);
        self.state = DurableMediaJobState::CancelRequested;
        Ok(JournalMutation::Applied)
    }

    pub fn acknowledge_cancel(
        &mut self,
        fence: MediaAttemptFence,
        now_ms: u64,
        publication_suppressed_and_staging_removed: bool,
    ) -> Result<JournalMutation, MediaServiceError> {
        self.require_fence(fence, now_ms)?;
        if self.state != DurableMediaJobState::CancelRequested
            || !publication_suppressed_and_staging_removed
        {
            return Err(MediaServiceError::CleanupUnconfirmed);
        }
        self.state = DurableMediaJobState::Cancelled;
        self.active_executor = None;
        self.active_fence = None;
        self.staged = None;
        Ok(JournalMutation::Applied)
    }

    pub fn mark_lease_ambiguous(&mut self, now_ms: u64) -> Result<(), MediaServiceError> {
        let fence = self.active_fence.ok_or(MediaServiceError::InvalidAttempt)?;
        if now_ms <= fence.lease_expires_at_ms
            || !matches!(
                self.state,
                DurableMediaJobState::Running
                    | DurableMediaJobState::Publishing
                    | DurableMediaJobState::CancelRequested
            )
        {
            return Err(MediaServiceError::InvalidTransition);
        }
        self.state = DurableMediaJobState::RecoveryRequired;
        Ok(())
    }

    pub fn validate_persisted(&self) -> Result<(), MediaServiceError> {
        if self.schema_version != MEDIA_JOB_JOURNAL_SCHEMA_VERSION
            || !canonical_uuid(&self.job_id)
            || media_service_catalog().get(self.kind).is_none()
            || self.next_attempt == 0
            || self.active_executor.is_some() != self.active_fence.is_some()
            || (self.state == DurableMediaJobState::Ready) != self.manifest.is_some()
            || (self.state == DurableMediaJobState::Publishing && self.staged.is_none())
            || (self.state.terminal() && self.active_fence.is_some())
        {
            return Err(MediaServiceError::InvalidJournal);
        }
        Ok(())
    }

    fn require_fence(
        &self,
        fence: MediaAttemptFence,
        now_ms: u64,
    ) -> Result<(), MediaServiceError> {
        if self.active_fence != Some(fence) || now_ms > fence.lease_expires_at_ms {
            return Err(MediaServiceError::LeaseLost);
        }
        Ok(())
    }

    fn validate_staged(
        &self,
        fence: MediaAttemptFence,
        staged: &StagedMediaOutput,
    ) -> Result<(), MediaServiceError> {
        let spec = media_service_catalog()
            .get(self.kind)
            .ok_or(MediaServiceError::UnknownJob)?;
        if staged.staging_key != self.artifact.staging_key(fence.attempt)?
            || staged.final_output_key != self.artifact.output_key
            || staged.bytes == 0
            || staged.bytes > spec.sandbox.max_output_bytes
            || !valid_sha256(&staged.checksum_sha256)
            || staged.content_type != self.artifact.content_type
            || !spec
                .output_content_types
                .contains(&staged.content_type.as_str())
            || staged.source_sha256 != self.artifact.source_sha256
            || staged.profile_sha256 != self.artifact.profile_sha256
            || staged.verified_at_ms == 0
        {
            return Err(MediaServiceError::UnverifiedOutput);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MediaArtifactHead {
    pub output_key: String,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
    pub source_sha256: String,
    pub profile_sha256: String,
}

impl fmt::Debug for MediaArtifactHead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaArtifactHead")
            .field("output_key", &"[redacted]")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"[redacted]")
            .field("content_type", &self.content_type)
            .field("source_sha256", &"[redacted]")
            .field("profile_sha256", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaArtifactObservation {
    Absent,
    StagingOnly(StagedMediaOutput),
    Committed(MediaArtifactHead),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaReconcileAction {
    Execute,
    ReuseCommitted,
    AdoptJournalVerifiedCommit,
    DeleteStagingThenRetry,
    DeleteAllAndSuppress,
    AwaitExecutorFence,
    QuarantineConflict,
    RaiseMissingCommittedArtifact,
}

impl DurableMediaJob {
    pub fn reconcile(
        &self,
        observation: &MediaArtifactObservation,
    ) -> Result<MediaReconcileAction, MediaServiceError> {
        self.validate_persisted()?;
        if matches!(
            self.state,
            DurableMediaJobState::CancelRequested | DurableMediaJobState::Cancelled
        ) {
            return Ok(match observation {
                MediaArtifactObservation::Absent => MediaReconcileAction::AwaitExecutorFence,
                _ => MediaReconcileAction::DeleteAllAndSuppress,
            });
        }
        if self.state == DurableMediaJobState::Ready {
            let manifest = self
                .manifest
                .as_ref()
                .ok_or(MediaServiceError::InvalidJournal)?;
            return Ok(match observation {
                MediaArtifactObservation::Committed(head)
                    if head_matches_manifest(head, manifest) =>
                {
                    MediaReconcileAction::ReuseCommitted
                }
                MediaArtifactObservation::Absent => {
                    MediaReconcileAction::RaiseMissingCommittedArtifact
                }
                _ => MediaReconcileAction::QuarantineConflict,
            });
        }
        match observation {
            MediaArtifactObservation::Absent => Ok(
                if self.state == DurableMediaJobState::RecoveryRequired
                    || self.active_fence.is_some()
                {
                    // One negative HEAD cannot prove that an acknowledged-late executor
                    // will not publish. A durable executor fence/cleanup receipt is still
                    // required before the reservation can be released.
                    MediaReconcileAction::AwaitExecutorFence
                } else {
                    MediaReconcileAction::Execute
                },
            ),
            MediaArtifactObservation::StagingOnly(staging) => {
                if self
                    .staged
                    .as_ref()
                    .is_none_or(|expected| expected == staging)
                {
                    Ok(MediaReconcileAction::DeleteStagingThenRetry)
                } else {
                    Ok(MediaReconcileAction::QuarantineConflict)
                }
            }
            MediaArtifactObservation::Committed(head) => {
                if self.staged.as_ref().is_some_and(|staged| {
                    head.output_key == staged.final_output_key
                        && head.bytes == staged.bytes
                        && head.checksum_sha256 == staged.checksum_sha256
                        && head.content_type == staged.content_type
                        && head.source_sha256 == staged.source_sha256
                        && head.profile_sha256 == staged.profile_sha256
                }) {
                    Ok(MediaReconcileAction::AdoptJournalVerifiedCommit)
                } else {
                    Ok(MediaReconcileAction::QuarantineConflict)
                }
            }
        }
    }

    pub fn close_recovery(
        &mut self,
        proof: MediaRecoveryClosureProof,
    ) -> Result<JournalMutation, MediaServiceError> {
        if self.state != DurableMediaJobState::RecoveryRequired
            && self.state != DurableMediaJobState::CancelRequested
        {
            return Err(MediaServiceError::InvalidTransition);
        }
        let fence = self.active_fence.ok_or(MediaServiceError::InvalidAttempt)?;
        if proof.attempt != fence.attempt
            || proof.lease_epoch != fence.lease_epoch
            || proof.observed_at_ms <= fence.lease_expires_at_ms
            || !proof.executor_fenced
            || !proof.staging_absent
            || !proof.final_absent
        {
            return Err(MediaServiceError::CleanupUnconfirmed);
        }
        let cancelled = self.cancel_requested_at_ms.is_some();
        self.active_executor = None;
        self.active_fence = None;
        self.staged = None;
        self.state = if cancelled {
            DurableMediaJobState::Cancelled
        } else {
            DurableMediaJobState::RetryableFailure
        };
        Ok(JournalMutation::Applied)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaRecoveryClosureProof {
    pub attempt: u16,
    pub lease_epoch: u64,
    pub observed_at_ms: u64,
    pub executor_fenced: bool,
    pub staging_absent: bool,
    pub final_absent: bool,
}

fn head_matches_manifest(head: &MediaArtifactHead, manifest: &MediaOutputManifest) -> bool {
    head.output_key == manifest.output_key
        && head.bytes == manifest.bytes
        && head.checksum_sha256 == manifest.checksum_sha256
        && head.content_type == manifest.content_type
        && head.source_sha256 == manifest.source_sha256
        && head.profile_sha256 == manifest.profile_sha256
}

#[derive(Clone, PartialEq, Eq)]
pub struct MediaExecutionRequest {
    pub kind: MediaJobKind,
    pub input: PrivateMediaInput,
    pub profile: MediaTransformProfile,
    pub artifact: MediaArtifactIdentity,
    pub fence: MediaAttemptFence,
}

impl fmt::Debug for MediaExecutionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaExecutionRequest")
            .field("kind", &self.kind)
            .field("input", &self.input)
            .field("profile", &self.profile)
            .field("artifact", &self.artifact)
            .field("fence", &self.fence)
            .finish()
    }
}

/// Provider-neutral derivative boundary. Implementations receive only a
/// tenant-scoped private object reference; URLs and credentials are absent by
/// construction. Publication remains a separate, journal-fenced operation.
pub trait MediaDerivativeExecutorPort: Send {
    fn executor(&self) -> MediaExecutorKind;
    fn supports(&self, kind: MediaJobKind) -> bool;
    fn head(&self, artifact: &MediaArtifactIdentity) -> Option<MediaArtifactHead>;
    fn execute(
        &mut self,
        request: &MediaExecutionRequest,
        now_ms: u64,
    ) -> Result<StagedMediaOutput, MediaServiceError>;
    fn publish(
        &mut self,
        request: &MediaExecutionRequest,
        staged: &StagedMediaOutput,
    ) -> Result<MediaArtifactHead, MediaServiceError>;
    fn cancel_and_cleanup(
        &mut self,
        request: &MediaExecutionRequest,
    ) -> Result<bool, MediaServiceError>;
}

pub struct OfflineMediaDerivativeExecutor {
    executor: MediaExecutorKind,
    supported: Vec<MediaJobKind>,
    staged: HashMap<String, StagedMediaOutput>,
    committed: HashMap<String, MediaArtifactHead>,
    injected_failure: Option<ExecutionFailureClass>,
}

impl fmt::Debug for OfflineMediaDerivativeExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OfflineMediaDerivativeExecutor")
            .field("executor", &self.executor)
            .field("supported", &self.supported)
            .field("staged_count", &self.staged.len())
            .field("committed_count", &self.committed.len())
            .field("injected_failure", &self.injected_failure)
            .finish()
    }
}

impl OfflineMediaDerivativeExecutor {
    #[must_use]
    pub fn new(
        executor: MediaExecutorKind,
        supported: impl IntoIterator<Item = MediaJobKind>,
    ) -> Self {
        Self {
            executor,
            supported: supported.into_iter().collect(),
            staged: HashMap::new(),
            committed: HashMap::new(),
            injected_failure: None,
        }
    }

    pub fn inject_failure(&mut self, failure: ExecutionFailureClass) {
        self.injected_failure = Some(failure);
    }
}

impl MediaDerivativeExecutorPort for OfflineMediaDerivativeExecutor {
    fn executor(&self) -> MediaExecutorKind {
        self.executor
    }

    fn supports(&self, kind: MediaJobKind) -> bool {
        self.supported.contains(&kind)
    }

    fn head(&self, artifact: &MediaArtifactIdentity) -> Option<MediaArtifactHead> {
        self.committed.get(&artifact.output_key).cloned()
    }

    fn execute(
        &mut self,
        request: &MediaExecutionRequest,
        now_ms: u64,
    ) -> Result<StagedMediaOutput, MediaServiceError> {
        request.input.validate()?;
        request.profile.validate_for(request.kind)?;
        if !self.supports(request.kind)
            || request.artifact
                != MediaArtifactIdentity::derive(&request.input, &request.profile, request.kind)?
        {
            return Err(MediaServiceError::UnsupportedExecutor);
        }
        if self.injected_failure.take().is_some() {
            return Err(MediaServiceError::ExecutorFailure);
        }
        let staging_key = request.artifact.staging_key(request.fence.attempt)?;
        if let Some(existing) = self.staged.get(&staging_key) {
            return Ok(existing.clone());
        }
        let mut digest = Sha256::new();
        digest.update(b"frame-offline-media-output-v1\0");
        digest.update(request.artifact.output_key.as_bytes());
        digest.update(request.fence.attempt.to_be_bytes());
        let staged = StagedMediaOutput {
            staging_key: staging_key.clone(),
            final_output_key: request.artifact.output_key.clone(),
            bytes: 1_024,
            checksum_sha256: hex(&digest.finalize()),
            content_type: request.artifact.content_type.clone(),
            source_sha256: request.artifact.source_sha256.clone(),
            profile_sha256: request.artifact.profile_sha256.clone(),
            verified_at_ms: now_ms,
        };
        self.staged.insert(staging_key, staged.clone());
        Ok(staged)
    }

    fn publish(
        &mut self,
        request: &MediaExecutionRequest,
        staged: &StagedMediaOutput,
    ) -> Result<MediaArtifactHead, MediaServiceError> {
        let head = MediaArtifactHead {
            output_key: staged.final_output_key.clone(),
            bytes: staged.bytes,
            checksum_sha256: staged.checksum_sha256.clone(),
            content_type: staged.content_type.clone(),
            source_sha256: staged.source_sha256.clone(),
            profile_sha256: staged.profile_sha256.clone(),
        };
        if let Some(existing) = self.committed.get(&head.output_key) {
            return if existing == &head {
                Ok(existing.clone())
            } else {
                Err(MediaServiceError::PublicationConflict)
            };
        }
        if self.staged.get(&staged.staging_key) != Some(staged)
            || staged.final_output_key != request.artifact.output_key
        {
            return Err(MediaServiceError::UnverifiedOutput);
        }
        self.committed.insert(head.output_key.clone(), head.clone());
        self.staged.remove(&staged.staging_key);
        Ok(head)
    }

    fn cancel_and_cleanup(
        &mut self,
        request: &MediaExecutionRequest,
    ) -> Result<bool, MediaServiceError> {
        let staging_key = request.artifact.staging_key(request.fence.attempt)?;
        self.staged.remove(&staging_key);
        Ok(!self.committed.contains_key(&request.artifact.output_key))
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MediaServiceError {
    #[error("media service catalog is incomplete or internally inconsistent")]
    InvalidCatalog,
    #[error("media service job is unknown")]
    UnknownJob,
    #[error("media service job identity is invalid")]
    InvalidJob,
    #[error("media input is malformed")]
    InvalidInput,
    #[error("media input did not come from a verified private object and probe")]
    UntrustedInput,
    #[error("media input role is invalid for the selected job")]
    InvalidInputRole,
    #[error("media transform profile is invalid for the selected job")]
    InvalidProfile,
    #[error("media capability limits are invalid")]
    InvalidLimits,
    #[error("media input or output exceeds a declared resource envelope")]
    ResourceLimit,
    #[error("managed fallback is forbidden for this failure")]
    FallbackForbidden,
    #[error("media artifact attempt is invalid")]
    InvalidAttempt,
    #[error("media job state transition is invalid")]
    InvalidTransition,
    #[error("media job lease was lost or expired")]
    LeaseLost,
    #[error("indeterminate progress cannot accept numeric updates")]
    IndeterminateProgress,
    #[error("media job progress cannot regress")]
    ProgressRegression,
    #[error("media execution cost overflowed")]
    CostOverflow,
    #[error("media execution exceeded its cost quota")]
    CostLimit,
    #[error("media publication was suppressed")]
    PublicationSuppressed,
    #[error("media publication conflicts with the logical result")]
    PublicationConflict,
    #[error("media output lacks journal-bound verification")]
    UnverifiedOutput,
    #[error("media cancellation or cleanup was not durably confirmed")]
    CleanupUnconfirmed,
    #[error("persisted media job state is invalid")]
    InvalidJournal,
    #[error("executor does not support the requested media contract")]
    UnsupportedExecutor,
    #[error("media executor failed with a redacted fault")]
    ExecutorFailure,
}

fn executor_allowed(spec: &MediaServiceJobSpec, executor: MediaExecutorKind) -> bool {
    match spec.disposition {
        MediaExecutionDisposition::HybridManagedNative => matches!(
            executor,
            MediaExecutorKind::CloudflareMedia | MediaExecutorKind::NativeGstreamer
        ),
        MediaExecutionDisposition::NativeOnly => executor == MediaExecutorKind::NativeGstreamer,
        MediaExecutionDisposition::ExternalProviderAdapter => {
            executor == MediaExecutorKind::ExternalProvider
        }
    }
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value != "00000000-0000-0000-0000-000000000000"
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
            }
        })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
    const VIDEO: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900103";
    const JOB: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900104";
    const SOURCE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn input() -> PrivateMediaInput {
        PrivateMediaInput {
            tenant_id: TENANT.into(),
            video_id: VIDEO.into(),
            object_key: format!("tenants/{TENANT}/videos/{VIDEO}/source/v1/payload.mp4"),
            role: MediaInputRole::SourceOriginal,
            source_version: 1,
            source_sha256: SOURCE_SHA.into(),
            metadata: MediaInput {
                bytes: 10_000_000,
                duration_ms: 60_000,
                width: 1_920,
                height: 1_080,
                container: ContainerFormat::Mp4,
                video_codec: VideoCodec::H264,
                audio_codec: AudioCodec::Aac,
                encrypted: false,
            },
            decoded_bytes_upper_bound: 4_000_000_000,
            frame_count_upper_bound: 1_800,
            track_count: 2,
            probe_trust: ProbeTrust::VerifiedNativeProbe,
        }
    }

    fn profile(kind: MediaJobKind) -> MediaTransformProfile {
        let (profile_id, mode, duration_ms, format, image_count, include_audio, max_output) =
            match kind {
                MediaJobKind::OptimizedClip => (
                    "optimized_clip_v1",
                    ManagedOutputMode::Video,
                    Some(10_000),
                    MediaOutputFormat::Mp4H264Aac,
                    None,
                    true,
                    32_000_000,
                ),
                MediaJobKind::Frame => (
                    "thumbnail_v1",
                    ManagedOutputMode::Frame,
                    None,
                    MediaOutputFormat::Jpeg,
                    None,
                    false,
                    4_000_000,
                ),
                MediaJobKind::Spritesheet => (
                    "spritesheet_v1",
                    ManagedOutputMode::Spritesheet,
                    Some(10_000),
                    MediaOutputFormat::Jpeg,
                    Some(10),
                    false,
                    16_000_000,
                ),
                MediaJobKind::AudioExtract => (
                    "audio_extract_v1",
                    ManagedOutputMode::Audio,
                    Some(10_000),
                    MediaOutputFormat::M4aAac,
                    None,
                    false,
                    16_000_000,
                ),
                _ => panic!("managed profile helper only accepts hybrid jobs"),
            };
        MediaTransformProfile {
            schema_version: MEDIA_TRANSFORM_PROFILE_SCHEMA_VERSION,
            profile_id: profile_id.into(),
            profile_version: 1,
            mode,
            start_ms: 0,
            duration_ms,
            width: if mode == ManagedOutputMode::Audio {
                None
            } else {
                Some(640)
            },
            height: if mode == ManagedOutputMode::Audio {
                None
            } else {
                Some(360)
            },
            fit: ResizeFit::ScaleDown,
            image_count,
            include_audio,
            format,
            max_output_bytes: max_output,
        }
    }

    fn router() -> MediaCapabilityRouter {
        MediaCapabilityRouter::new(
            true,
            ManagedMediaContract::cloudflare_2026_06_10(),
            NativeMediaContract {
                limits: HEAVY_NATIVE,
                max_decompression_ratio: 1_000,
            },
        )
        .expect("valid router")
    }

    fn artifact() -> MediaArtifactIdentity {
        MediaArtifactIdentity::derive(&input(), &profile(MediaJobKind::Frame), MediaJobKind::Frame)
            .expect("artifact")
    }

    fn fence(job: &mut DurableMediaJob) -> MediaAttemptFence {
        job.claim(
            MediaExecutorKind::CloudflareMedia,
            1,
            100,
            1_000,
            b"0123456789abcdef0123456789abcdef",
        )
        .expect("claim")
    }

    fn staged(artifact: &MediaArtifactIdentity, fence: MediaAttemptFence) -> StagedMediaOutput {
        StagedMediaOutput {
            staging_key: artifact.staging_key(fence.attempt).expect("staging key"),
            final_output_key: artifact.output_key.clone(),
            bytes: 1_024,
            checksum_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into(),
            content_type: artifact.content_type.clone(),
            source_sha256: artifact.source_sha256.clone(),
            profile_sha256: artifact.profile_sha256.clone(),
            verified_at_ms: 300,
        }
    }

    #[test]
    fn catalog_is_exact_complete_and_declares_every_cap_disposition() {
        let catalog = media_service_catalog().validate().expect("catalog");
        assert_eq!(catalog.jobs.len(), 16);
        for kind in MediaJobKind::ALL {
            let job = catalog.get(kind).expect("one catalog entry");
            assert!(!job.cap_sources.is_empty());
            assert!(!job.output_content_types.is_empty());
            assert_ne!(job.timeout_ms, 0);
            assert_ne!(job.max_attempts, 0);
        }
        assert_eq!(
            catalog
                .get(MediaJobKind::Transcription)
                .expect("job")
                .disposition,
            MediaExecutionDisposition::ExternalProviderAdapter
        );
        assert_eq!(
            catalog
                .get(MediaJobKind::Composition)
                .expect("job")
                .disposition,
            MediaExecutionDisposition::NativeOnly
        );
    }

    #[test]
    fn every_managed_mode_routes_only_inside_exact_contract() {
        for kind in [
            MediaJobKind::OptimizedClip,
            MediaJobKind::Frame,
            MediaJobKind::Spritesheet,
            MediaJobKind::AudioExtract,
        ] {
            let decision = router()
                .route(kind, &input(), Some(&profile(kind)))
                .expect("managed route");
            assert_eq!(
                decision.executor,
                MediaExecutorKind::CloudflareMedia,
                "{kind:?}: {:?}",
                decision.reason
            );
            assert!(decision.fallback_available);
        }

        let mut exact = input();
        exact.metadata.bytes = 99_999_999;
        exact.decoded_bytes_upper_bound = 64_000_000_000;
        exact.metadata.duration_ms = 600_000;
        let mut exact_profile = profile(MediaJobKind::Frame);
        // Binding time strings are emitted in whole seconds. The final
        // representable instant before the ten-minute source end is 599s.
        exact_profile.start_ms = 599_000;
        exact_profile.width = Some(2_000);
        exact_profile.height = Some(2_000);
        assert_eq!(
            router()
                .route(MediaJobKind::Frame, &exact, Some(&exact_profile))
                .expect("exact supported edge")
                .executor,
            MediaExecutorKind::CloudflareMedia
        );

        let mut over = exact.clone();
        over.metadata.bytes = 100_000_000;
        assert_eq!(
            router()
                .route(MediaJobKind::Frame, &over, Some(&exact_profile))
                .expect("native fallback")
                .reason,
            MediaServiceRouteReason::ManagedInputLimit
        );
        let mut long = profile(MediaJobKind::OptimizedClip);
        long.duration_ms = Some(60_001);
        assert_eq!(
            router()
                .route(MediaJobKind::OptimizedClip, &input(), Some(&long))
                .expect("native fallback")
                .reason,
            MediaServiceRouteReason::ManagedProfileLimit
        );
    }

    #[test]
    fn exact_and_just_over_limits_are_covered_for_each_managed_capability() {
        for kind in [
            MediaJobKind::OptimizedClip,
            MediaJobKind::Frame,
            MediaJobKind::Spritesheet,
            MediaJobKind::AudioExtract,
        ] {
            let mut edge_input = input();
            edge_input.metadata.bytes = 99_999_999;
            edge_input.metadata.duration_ms = 600_000;
            edge_input.decoded_bytes_upper_bound = 64_000_000_000;
            let mut edge_profile = profile(kind);
            edge_profile.start_ms = if kind == MediaJobKind::Frame {
                599_000
            } else {
                540_000
            };
            if edge_profile.duration_ms.is_some() {
                edge_profile.duration_ms = Some(60_000);
            }
            if edge_profile.width.is_some() {
                edge_profile.width = Some(2_000);
                edge_profile.height = Some(2_000);
            }
            assert_eq!(
                router()
                    .route(kind, &edge_input, Some(&edge_profile))
                    .expect("exact managed edge")
                    .executor,
                MediaExecutorKind::CloudflareMedia,
                "{kind:?}"
            );

            let mut too_large = edge_input.clone();
            too_large.metadata.bytes = 100_000_000;
            assert_eq!(
                router()
                    .route(kind, &too_large, Some(&edge_profile))
                    .expect("byte fallback")
                    .reason,
                MediaServiceRouteReason::ManagedInputLimit,
                "{kind:?}"
            );

            let mut too_long = edge_input.clone();
            too_long.metadata.duration_ms = 600_001;
            assert_eq!(
                router()
                    .route(kind, &too_long, Some(&edge_profile))
                    .expect("duration fallback")
                    .reason,
                MediaServiceRouteReason::ManagedInputLimit,
                "{kind:?}"
            );

            if edge_profile.width.is_some() {
                let mut too_wide = edge_profile.clone();
                too_wide.width = Some(2_001);
                assert_eq!(
                    router()
                        .route(kind, &edge_input, Some(&too_wide))
                        .expect("dimension fallback")
                        .reason,
                    MediaServiceRouteReason::ManagedProfileLimit,
                    "{kind:?}"
                );
            }

            if edge_profile.duration_ms.is_some() {
                let mut too_long_output = edge_profile.clone();
                too_long_output.duration_ms = Some(60_001);
                assert_eq!(
                    router()
                        .route(kind, &edge_input, Some(&too_long_output))
                        .expect("output duration fallback")
                        .reason,
                    MediaServiceRouteReason::ManagedProfileLimit,
                    "{kind:?}"
                );
            }

            let managed_contract = ManagedMediaContract::cloudflare_2026_06_10();
            let output_ceiling = managed_contract.max_output_bytes(edge_profile.mode);
            let mut exact_output = edge_profile.clone();
            exact_output.max_output_bytes = output_ceiling;
            assert_eq!(
                router()
                    .route(kind, &edge_input, Some(&exact_output))
                    .expect("exact output limit")
                    .executor,
                MediaExecutorKind::CloudflareMedia,
                "{kind:?}"
            );
            let mut over_output = exact_output;
            over_output.max_output_bytes = output_ceiling + 1;
            assert_eq!(
                router()
                    .route(kind, &edge_input, Some(&over_output))
                    .expect("output size fallback")
                    .reason,
                MediaServiceRouteReason::ManagedProfileLimit,
                "{kind:?}"
            );
        }

        let mut exact_sprites = profile(MediaJobKind::Spritesheet);
        exact_sprites.image_count = Some(100);
        assert_eq!(
            router()
                .route(MediaJobKind::Spritesheet, &input(), Some(&exact_sprites))
                .expect("exact image count")
                .executor,
            MediaExecutorKind::CloudflareMedia
        );
        exact_sprites.image_count = Some(101);
        assert_eq!(
            router()
                .route(MediaJobKind::Spritesheet, &input(), Some(&exact_sprites))
                .expect("image count fallback")
                .reason,
            MediaServiceRouteReason::ManagedProfileLimit
        );

        let mut subsecond = profile(MediaJobKind::Frame);
        subsecond.start_ms = 1;
        assert_eq!(
            router()
                .route(MediaJobKind::Frame, &input(), Some(&subsecond))
                .expect("unsupported binding time precision routes native")
                .reason,
            MediaServiceRouteReason::ManagedProfileLimit
        );
        let mut subsecond = profile(MediaJobKind::OptimizedClip);
        subsecond.duration_ms = Some(1_001);
        assert_eq!(
            router()
                .route(MediaJobKind::OptimizedClip, &input(), Some(&subsecond))
                .expect("unsupported binding duration precision routes native")
                .reason,
            MediaServiceRouteReason::ManagedProfileLimit
        );
    }

    #[test]
    fn fallback_matrix_retries_only_safe_managed_failures() {
        let input = input();
        let profile = profile(MediaJobKind::Frame);
        let managed = router()
            .route(MediaJobKind::Frame, &input, Some(&profile))
            .expect("managed");
        for failure in HYBRID_RETRYABLE {
            let fallback = router()
                .fallback_after_failure(managed, *failure, &input, &profile)
                .expect("safe fallback");
            assert_eq!(fallback.executor, MediaExecutorKind::NativeGstreamer);
            assert_eq!(
                fallback.reason,
                MediaServiceRouteReason::ManagedFailure(*failure)
            );
        }
        for failure in [
            ExecutionFailureClass::InvalidInput,
            ExecutionFailureClass::SecurityViolation,
            ExecutionFailureClass::Cancelled,
        ] {
            assert_eq!(
                router().fallback_after_failure(managed, failure, &input, &profile),
                Err(MediaServiceError::FallbackForbidden)
            );
        }
    }

    #[test]
    fn private_source_and_probe_are_tenant_bound_and_url_free() {
        let mut hostile = input();
        hostile.object_key = "https://user:secret@example.com/source.mp4".into();
        assert_eq!(hostile.validate(), Err(MediaServiceError::InvalidInput));

        let mut cross_tenant = input();
        cross_tenant.object_key = format!(
            "tenants/018f47a6-7b1c-7f55-8f39-8f8a86909999/videos/{VIDEO}/source/v1/payload.mp4"
        );
        assert_eq!(
            cross_tenant.validate(),
            Err(MediaServiceError::UntrustedInput)
        );

        let mut caller_claim = input();
        caller_claim.probe_trust = ProbeTrust::UntrustedCaller;
        assert_eq!(
            caller_claim.validate(),
            Err(MediaServiceError::UntrustedInput)
        );
        assert!(!format!("{:?}", input()).contains(TENANT));
        assert!(!format!("{:?}", input()).contains("payload.mp4"));
    }

    #[test]
    fn unsupported_format_and_decompression_bomb_never_escape_declared_routes() {
        let mut unsupported = input();
        unsupported.metadata.video_codec = VideoCodec::Av1;
        assert_eq!(
            router()
                .route(
                    MediaJobKind::OptimizedClip,
                    &unsupported,
                    Some(&profile(MediaJobKind::OptimizedClip))
                )
                .expect("native route")
                .reason,
            MediaServiceRouteReason::ManagedInputFormat
        );

        let mut bomb = input();
        bomb.metadata.container = ContainerFormat::Matroska;
        bomb.metadata.video_codec = VideoCodec::Vp9;
        bomb.decoded_bytes_upper_bound = HEAVY_NATIVE.max_decoded_bytes + 1;
        assert_eq!(
            router().route(MediaJobKind::Probe, &bomb, None),
            Err(MediaServiceError::ResourceLimit)
        );
    }

    #[test]
    fn input_role_and_native_progress_are_fenced() {
        let mut wrong_role = input();
        wrong_role.role = MediaInputRole::ExtractedAudio;
        assert_eq!(
            router().route(
                MediaJobKind::Frame,
                &wrong_role,
                Some(&profile(MediaJobKind::Frame))
            ),
            Err(MediaServiceError::InvalidInputRole)
        );

        let mut composition_input = input();
        composition_input.role = MediaInputRole::EditTimeline;
        let profile_digest = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let artifact = MediaArtifactIdentity::derive_declared(
            &composition_input,
            MediaJobKind::Composition,
            1,
            profile_digest,
            "video/mp4",
            "mp4",
        )
        .expect("native artifact");
        let mut job = DurableMediaJob::new(JOB, MediaJobKind::Composition, artifact)
            .expect("composition job");
        let fence = job
            .claim(
                MediaExecutorKind::NativeGstreamer,
                3,
                100,
                1_000,
                b"0123456789abcdef0123456789abcdef",
            )
            .expect("claim");
        assert_eq!(
            job.record_progress(fence, 200, 5_000).expect("progress"),
            JournalMutation::Applied
        );
        assert_eq!(
            job.record_progress(fence, 201, 4_999),
            Err(MediaServiceError::ProgressRegression)
        );
    }

    #[test]
    fn deterministic_adversarial_preflight_has_no_fail_open_or_panic_path() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..10_000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let mut candidate = input();
            candidate.metadata.bytes = seed & ((1_u64 << 40) - 1);
            candidate.metadata.duration_ms = seed.rotate_left(7) & ((1_u64 << 32) - 1);
            candidate.metadata.width = (seed.rotate_left(13) & 0xffff) as u32;
            candidate.metadata.height = (seed.rotate_left(29) & 0xffff) as u32;
            candidate.decoded_bytes_upper_bound = seed.rotate_left(37);
            candidate.frame_count_upper_bound = seed.rotate_left(43) & ((1_u64 << 32) - 1);
            candidate.track_count = (seed.rotate_left(53) & 0xffff) as u16;
            let result = router().route(
                MediaJobKind::Frame,
                &candidate,
                Some(&profile(MediaJobKind::Frame)),
            );
            if let Ok(decision) = result {
                assert!(matches!(
                    decision.executor,
                    MediaExecutorKind::CloudflareMedia | MediaExecutorKind::NativeGstreamer
                ));
            }
        }
    }

    #[test]
    fn immutable_identity_binds_source_profile_and_output_format() {
        let first = artifact();
        let replay = artifact();
        assert_eq!(first, replay);
        assert!(first.output_key.starts_with(&format!(
            "tenants/{TENANT}/videos/{VIDEO}/derivatives/thumbnail_v1/v1/"
        )));
        let mut changed = profile(MediaJobKind::Frame);
        changed.width = Some(1_280);
        changed.height = Some(720);
        let changed = MediaArtifactIdentity::derive(&input(), &changed, MediaJobKind::Frame)
            .expect("changed identity");
        assert_ne!(first.output_key, changed.output_key);
        assert!(!format!("{first:?}").contains(TENANT));
    }

    #[test]
    fn durable_job_enforces_progress_publication_and_one_logical_result() {
        let artifact = artifact();
        let mut job =
            DurableMediaJob::new(JOB, MediaJobKind::Frame, artifact.clone()).expect("new job");
        let fence = fence(&mut job);
        assert_eq!(
            job.record_progress(fence, 200, 5_000),
            Err(MediaServiceError::IndeterminateProgress)
        );
        let staged = staged(&artifact, fence);
        job.record_staged(fence, 300, staged.clone())
            .expect("stage");
        let manifest = MediaOutputManifest {
            schema_version: MEDIA_JOB_JOURNAL_SCHEMA_VERSION,
            output_key: artifact.output_key.clone(),
            manifest_key: artifact.manifest_key.clone(),
            bytes: staged.bytes,
            checksum_sha256: staged.checksum_sha256.clone(),
            content_type: staged.content_type.clone(),
            source_sha256: staged.source_sha256.clone(),
            profile_sha256: staged.profile_sha256.clone(),
            executor: MediaExecutorKind::CloudflareMedia,
            attempt: fence.attempt,
            committed_at_ms: 400,
            verification: MediaOutputVerification {
                playable: true,
                metadata_passed: true,
                perceptual_passed: true,
                waveform_passed: true,
                caption_timing_passed: true,
            },
            cost: MediaExecutionCost::default(),
        };
        assert_eq!(
            job.commit(fence, 400, manifest.clone()).expect("commit"),
            JournalMutation::Applied
        );
        assert_eq!(
            job.commit(fence, 400, manifest.clone()).expect("replay"),
            JournalMutation::Replayed
        );
        let mut conflict = manifest;
        conflict.bytes += 1;
        assert_eq!(
            job.commit(fence, 400, conflict),
            Err(MediaServiceError::PublicationConflict)
        );
        assert_eq!(job.state, DurableMediaJobState::Ready);
        assert_eq!(job.progress_basis_points, Some(10_000));
    }

    #[test]
    fn ambiguous_execution_retains_ownership_until_fenced_cleanup_proof() {
        let artifact = artifact();
        let mut job = DurableMediaJob::new(JOB, MediaJobKind::Frame, artifact).expect("new job");
        let fence = fence(&mut job);
        job.mark_lease_ambiguous(1_001).expect("expired ambiguity");
        assert_eq!(
            job.reconcile(&MediaArtifactObservation::Absent)
                .expect("reconcile"),
            MediaReconcileAction::AwaitExecutorFence
        );
        assert_eq!(
            job.close_recovery(MediaRecoveryClosureProof {
                attempt: fence.attempt,
                lease_epoch: fence.lease_epoch,
                observed_at_ms: 1_002,
                executor_fenced: false,
                staging_absent: true,
                final_absent: true,
            }),
            Err(MediaServiceError::CleanupUnconfirmed)
        );
        job.close_recovery(MediaRecoveryClosureProof {
            attempt: fence.attempt,
            lease_epoch: fence.lease_epoch,
            observed_at_ms: 1_002,
            executor_fenced: true,
            staging_absent: true,
            final_absent: true,
        })
        .expect("fenced recovery");
        assert_eq!(job.state, DurableMediaJobState::RetryableFailure);
    }

    #[test]
    fn cancellation_never_publishes_and_requires_cleanup_confirmation() {
        let artifact = artifact();
        let mut job =
            DurableMediaJob::new(JOB, MediaJobKind::Frame, artifact.clone()).expect("new job");
        let fence = fence(&mut job);
        job.request_cancel(200).expect("cancel request");
        assert_eq!(
            job.record_staged(fence, 300, staged(&artifact, fence)),
            Err(MediaServiceError::PublicationSuppressed)
        );
        assert_eq!(
            job.acknowledge_cancel(fence, 300, false),
            Err(MediaServiceError::CleanupUnconfirmed)
        );
        job.acknowledge_cancel(fence, 300, true)
            .expect("cleanup confirmation");
        assert_eq!(job.state, DurableMediaJobState::Cancelled);
    }

    #[test]
    fn offline_port_heads_reuses_and_atomically_publishes_deterministic_result() {
        let input = input();
        let profile = profile(MediaJobKind::Frame);
        let artifact =
            MediaArtifactIdentity::derive(&input, &profile, MediaJobKind::Frame).expect("artifact");
        let mut job =
            DurableMediaJob::new(JOB, MediaJobKind::Frame, artifact.clone()).expect("job");
        let fence = fence(&mut job);
        let request = MediaExecutionRequest {
            kind: MediaJobKind::Frame,
            input,
            profile,
            artifact: artifact.clone(),
            fence,
        };
        let mut fake = OfflineMediaDerivativeExecutor::new(
            MediaExecutorKind::CloudflareMedia,
            [MediaJobKind::Frame],
        );
        let first = fake.execute(&request, 200).expect("execute");
        assert_eq!(fake.execute(&request, 200).expect("replay"), first);
        assert!(fake.head(&artifact).is_none());
        let published = fake.publish(&request, &first).expect("publish");
        assert_eq!(fake.head(&artifact), Some(published.clone()));
        assert_eq!(
            fake.publish(&request, &first).expect("publish replay"),
            published
        );
        assert!(
            !fake
                .cancel_and_cleanup(&request)
                .expect("cancel after publish")
        );
    }
}
