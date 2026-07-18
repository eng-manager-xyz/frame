use std::{
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(all(not(test), unix))]
use std::{
    process::{Command, Stdio},
    thread,
};

#[cfg(test)]
use frame_media::MediaExecutionDisposition;
use frame_media::{
    CancellationToken, MEDIA_JOB_CATALOG_VERSION, MEDIA_SERVICE_CATALOG_VERSION, MediaJobKind,
    MediaSandboxLimits, NetworkPolicy, diagnose_runtime, media_service_catalog,
    pipeline_has_trusted_factory_provenance, runtime_manifest,
};
use gst::prelude::*;
use gstreamer as gst;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const NATIVE_EXECUTION_PLAN_SCHEMA_VERSION: u16 = 1;
const MAX_PLAN_BYTES: u64 = 64 * 1_024;
const MAX_ANALYSIS_JSON_BYTES: u64 = 4 * 1_024 * 1_024;
const BUS_POLL: Duration = Duration::from_millis(50);
const MAX_WAVEFORM_POINTS: usize = 4_096;
const MAX_PROBE_FRAME_RATE: u64 = 240;
const MAX_AUDIO_SAMPLE_RATE: u64 = 192_000;
const MAX_AUDIO_CHANNELS: u64 = 8;
const MAX_AUDIO_SAMPLE_BYTES: u64 = 4;
const THUMBNAIL_WIDTH: u32 = 640;
const THUMBNAIL_HEIGHT: u32 = 360;
const H264_AAC_APPROVAL: &str = "approved-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeExecutionOriginV1 {
    NativeOnly,
    ManagedFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeImplementationEngineV1 {
    Gstreamer,
    RustWithGstreamerDecode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeImplementationStateV1 {
    Executable,
    ExecutableWithVariantException,
    DocumentedException,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeImplementationExceptionV1 {
    JpegThumbnailGraphNotAudited,
    H264AacCodecApproval,
    SpritesheetSamplingContract,
    AnimatedPreviewSamplingContract,
    LoudnessAlgorithmApproval,
    ContainerDemuxAllowlist,
    SegmentMuxGraphNotAudited,
    CompositionTimelineProtocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeImplementationSpecV1 {
    pub(crate) graph_id: &'static str,
    pub(crate) engine: NativeImplementationEngineV1,
    pub(crate) state: NativeImplementationStateV1,
    pub(crate) implemented_output_content_types: &'static [&'static str],
    pub(crate) required_factories: &'static [&'static str],
    pub(crate) exception: Option<NativeImplementationExceptionV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NativeProfile {
    #[serde(rename = "optimized_clip_v1")]
    OptimizedClip,
    #[serde(rename = "thumbnail_v1")]
    Frame,
    #[serde(rename = "spritesheet_v1")]
    Spritesheet,
    #[serde(rename = "audio_extract_v1")]
    AudioExtract,
    #[serde(rename = "probe_v1")]
    Probe,
    #[serde(rename = "audio_presence_v1")]
    AudioPresence,
    #[serde(rename = "distribution_master_v1")]
    DistributionMaster,
    #[serde(rename = "animated_preview_v1")]
    AnimatedPreview,
    #[serde(rename = "audio_normalize_v1")]
    AudioNormalize,
    #[serde(rename = "remux_repair_v1")]
    RemuxRepair,
    #[serde(rename = "segment_mux_v1")]
    SegmentMux,
    #[serde(rename = "waveform_v1")]
    Waveform,
    #[serde(rename = "composition_v1")]
    Composition,
    #[serde(rename = "normalize_v1")]
    Normalize,
}

impl NativeProfile {
    pub(crate) const ALL: [Self; 14] = [
        Self::OptimizedClip,
        Self::Frame,
        Self::Spritesheet,
        Self::AudioExtract,
        Self::Probe,
        Self::AudioPresence,
        Self::DistributionMaster,
        Self::AnimatedPreview,
        Self::AudioNormalize,
        Self::RemuxRepair,
        Self::SegmentMux,
        Self::Waveform,
        Self::Composition,
        Self::Normalize,
    ];

    pub(crate) fn parse(profile_id: &str) -> Result<Self, NativeError> {
        match profile_id {
            "optimized_clip_v1" => Ok(Self::OptimizedClip),
            "thumbnail_v1" => Ok(Self::Frame),
            "spritesheet_v1" => Ok(Self::Spritesheet),
            "audio_extract_v1" => Ok(Self::AudioExtract),
            "probe_v1" => Ok(Self::Probe),
            "audio_presence_v1" => Ok(Self::AudioPresence),
            "distribution_master_v1" => Ok(Self::DistributionMaster),
            "animated_preview_v1" => Ok(Self::AnimatedPreview),
            "audio_normalize_v1" => Ok(Self::AudioNormalize),
            "remux_repair_v1" => Ok(Self::RemuxRepair),
            "segment_mux_v1" => Ok(Self::SegmentMux),
            "waveform_v1" => Ok(Self::Waveform),
            "composition_v1" => Ok(Self::Composition),
            "normalize_v1" => Ok(Self::Normalize),
            _ => Err(NativeError::UnsupportedProfile),
        }
    }

    #[cfg(test)]
    pub(crate) const fn profile_id(self) -> &'static str {
        match self {
            Self::OptimizedClip => "optimized_clip_v1",
            Self::Frame => "thumbnail_v1",
            Self::Spritesheet => "spritesheet_v1",
            Self::AudioExtract => "audio_extract_v1",
            Self::Probe => "probe_v1",
            Self::AudioPresence => "audio_presence_v1",
            Self::DistributionMaster => "distribution_master_v1",
            Self::AnimatedPreview => "animated_preview_v1",
            Self::AudioNormalize => "audio_normalize_v1",
            Self::RemuxRepair => "remux_repair_v1",
            Self::SegmentMux => "segment_mux_v1",
            Self::Waveform => "waveform_v1",
            Self::Composition => "composition_v1",
            Self::Normalize => "normalize_v1",
        }
    }

    pub(crate) const fn job_kind(self) -> MediaJobKind {
        match self {
            Self::OptimizedClip => MediaJobKind::OptimizedClip,
            Self::Frame => MediaJobKind::Frame,
            Self::Spritesheet => MediaJobKind::Spritesheet,
            Self::AudioExtract => MediaJobKind::AudioExtract,
            Self::Probe => MediaJobKind::Probe,
            Self::AudioPresence => MediaJobKind::AudioPresence,
            Self::DistributionMaster => MediaJobKind::DistributionMaster,
            Self::AnimatedPreview => MediaJobKind::AnimatedPreview,
            Self::AudioNormalize => MediaJobKind::AudioNormalize,
            Self::RemuxRepair => MediaJobKind::RemuxRepair,
            Self::SegmentMux => MediaJobKind::SegmentMux,
            Self::Waveform => MediaJobKind::Waveform,
            Self::Composition => MediaJobKind::Composition,
            Self::Normalize => MediaJobKind::Normalize,
        }
    }

    pub(crate) const fn expected_origin(self) -> NativeExecutionOriginV1 {
        match self {
            Self::OptimizedClip | Self::Frame | Self::Spritesheet | Self::AudioExtract => {
                NativeExecutionOriginV1::ManagedFallback
            }
            Self::Probe
            | Self::AudioPresence
            | Self::DistributionMaster
            | Self::AnimatedPreview
            | Self::AudioNormalize
            | Self::RemuxRepair
            | Self::SegmentMux
            | Self::Waveform
            | Self::Composition
            | Self::Normalize => NativeExecutionOriginV1::NativeOnly,
        }
    }

    pub(crate) const fn source_count(self) -> std::ops::RangeInclusive<usize> {
        match self {
            Self::SegmentMux => 2..=64,
            Self::Composition => 1..=64,
            _ => 1..=1,
        }
    }

    pub(crate) const fn output_role(self) -> &'static str {
        match self {
            Self::OptimizedClip => "preview",
            Self::Frame => "thumbnail",
            Self::Spritesheet => "spritesheet",
            Self::AudioExtract => "extracted_audio",
            Self::Probe | Self::AudioPresence => "probe_manifest",
            Self::DistributionMaster => "distribution_master",
            Self::AnimatedPreview => "animated_preview",
            Self::AudioNormalize => "normalized_audio",
            Self::RemuxRepair => "repaired_media",
            Self::SegmentMux => "muxed_media",
            Self::Waveform => "waveform",
            Self::Composition => "composition",
            Self::Normalize => "normalized_media",
        }
    }

    pub(crate) const fn implementation(self) -> NativeImplementationSpecV1 {
        const FRAME: &[&str] = &[
            "filesrc",
            "decodebin",
            "videoconvert",
            "videoscale",
            "pngenc",
            "filesink",
        ];
        const ANALYSIS: &[&str] = &["filesrc", "decodebin", "queue", "fakesink"];
        const WAVEFORM: &[&str] = &[
            "filesrc",
            "decodebin",
            "queue",
            "audioconvert",
            "audioresample",
            "capsfilter",
            "identity",
            "fakesink",
        ];
        const VIDEO_TRANSCODE: &[&str] = &[
            "filesrc",
            "decodebin",
            "queue",
            "videoconvert",
            "videoscale",
            "x264enc",
            "h264parse",
            "audioconvert",
            "audioresample",
            "avenc_aac",
            "aacparse",
            "mp4mux",
            "filesink",
        ];
        const AUDIO_EXTRACT: &[&str] = &[
            "filesrc",
            "decodebin",
            "queue",
            "audioconvert",
            "audioresample",
            "avenc_aac",
            "aacparse",
            "mp4mux",
            "filesink",
        ];
        const SPRITESHEET: &[&str] = &[
            "filesrc",
            "decodebin",
            "videoconvert",
            "videoscale",
            "videorate",
            "compositor",
            "jpegenc",
            "filesink",
        ];
        const ANIMATED_PREVIEW: &[&str] = &[
            "filesrc",
            "decodebin",
            "videoconvert",
            "videoscale",
            "videorate",
            "gifenc",
            "filesink",
        ];
        const AUDIO_NORMALIZE: &[&str] = &[
            "filesrc",
            "decodebin",
            "queue",
            "audioconvert",
            "audioresample",
            "volume",
            "wavenc",
            "filesink",
        ];
        const REMUX: &[&str] = &["filesrc", "h264parse", "aacparse", "mp4mux", "filesink"];
        const SEGMENT_MUX: &[&str] = &[
            "filesrc",
            "concat",
            "h264parse",
            "aacparse",
            "mp4mux",
            "filesink",
        ];
        const COMPOSITION: &[&str] = &[
            "filesrc",
            "decodebin",
            "queue",
            "compositor",
            "audiomixer",
            "videoconvert",
            "audioconvert",
            "audioresample",
            "x264enc",
            "avenc_aac",
            "h264parse",
            "aacparse",
            "mp4mux",
            "filesink",
        ];
        match self {
            Self::OptimizedClip => NativeImplementationSpecV1 {
                graph_id: "optimized_clip_h264_aac_mp4_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: VIDEO_TRANSCODE,
                exception: Some(NativeImplementationExceptionV1::H264AacCodecApproval),
            },
            Self::Frame => NativeImplementationSpecV1 {
                graph_id: "thumbnail_png_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::ExecutableWithVariantException,
                implemented_output_content_types: &["image/png"],
                required_factories: FRAME,
                exception: Some(NativeImplementationExceptionV1::JpegThumbnailGraphNotAudited),
            },
            Self::Spritesheet => NativeImplementationSpecV1 {
                graph_id: "sampled_contact_sheet_jpeg_v1",
                engine: NativeImplementationEngineV1::RustWithGstreamerDecode,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: SPRITESHEET,
                exception: Some(NativeImplementationExceptionV1::SpritesheetSamplingContract),
            },
            Self::AudioExtract => NativeImplementationSpecV1 {
                graph_id: "audio_extract_aac_m4a_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: AUDIO_EXTRACT,
                exception: Some(NativeImplementationExceptionV1::H264AacCodecApproval),
            },
            Self::Probe => NativeImplementationSpecV1 {
                graph_id: "probe_manifest_v1",
                engine: NativeImplementationEngineV1::RustWithGstreamerDecode,
                state: NativeImplementationStateV1::Executable,
                implemented_output_content_types: &["application/json"],
                required_factories: ANALYSIS,
                exception: None,
            },
            Self::AudioPresence => NativeImplementationSpecV1 {
                graph_id: "audio_presence_manifest_v1",
                engine: NativeImplementationEngineV1::RustWithGstreamerDecode,
                state: NativeImplementationStateV1::Executable,
                implemented_output_content_types: &["application/json"],
                required_factories: ANALYSIS,
                exception: None,
            },
            Self::DistributionMaster => NativeImplementationSpecV1 {
                graph_id: "distribution_master_h264_aac_mp4_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: VIDEO_TRANSCODE,
                exception: Some(NativeImplementationExceptionV1::H264AacCodecApproval),
            },
            Self::AnimatedPreview => NativeImplementationSpecV1 {
                graph_id: "sampled_animated_preview_gif_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: ANIMATED_PREVIEW,
                exception: Some(NativeImplementationExceptionV1::AnimatedPreviewSamplingContract),
            },
            Self::AudioNormalize => NativeImplementationSpecV1 {
                graph_id: "two_pass_audio_normalize_v1",
                engine: NativeImplementationEngineV1::RustWithGstreamerDecode,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: AUDIO_NORMALIZE,
                exception: Some(NativeImplementationExceptionV1::LoudnessAlgorithmApproval),
            },
            Self::RemuxRepair => NativeImplementationSpecV1 {
                graph_id: "allowlisted_remux_repair_mp4_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: REMUX,
                exception: Some(NativeImplementationExceptionV1::ContainerDemuxAllowlist),
            },
            Self::SegmentMux => NativeImplementationSpecV1 {
                graph_id: "ordered_segment_mux_mp4_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: SEGMENT_MUX,
                exception: Some(NativeImplementationExceptionV1::SegmentMuxGraphNotAudited),
            },
            Self::Waveform => NativeImplementationSpecV1 {
                graph_id: "bounded_waveform_manifest_v1",
                engine: NativeImplementationEngineV1::RustWithGstreamerDecode,
                state: NativeImplementationStateV1::Executable,
                implemented_output_content_types: &["application/json"],
                required_factories: WAVEFORM,
                exception: None,
            },
            Self::Composition => NativeImplementationSpecV1 {
                graph_id: "timeline_composition_h264_aac_mp4_v1",
                engine: NativeImplementationEngineV1::RustWithGstreamerDecode,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: COMPOSITION,
                exception: Some(NativeImplementationExceptionV1::CompositionTimelineProtocol),
            },
            Self::Normalize => NativeImplementationSpecV1 {
                graph_id: "normalized_h264_aac_mp4_v1",
                engine: NativeImplementationEngineV1::Gstreamer,
                state: NativeImplementationStateV1::DocumentedException,
                implemented_output_content_types: &[],
                required_factories: VIDEO_TRANSCODE,
                exception: Some(NativeImplementationExceptionV1::H264AacCodecApproval),
            },
        }
    }

    pub(crate) const fn has_implemented_graph(self) -> bool {
        matches!(
            self.implementation().state,
            NativeImplementationStateV1::Executable
                | NativeImplementationStateV1::ExecutableWithVariantException
        )
    }

    pub(crate) fn sandbox(self) -> Result<NativeSandboxEnvelopeV1, NativeError> {
        let spec = media_service_catalog()
            .get(self.job_kind())
            .ok_or(NativeError::UnsupportedProfile)?;
        spec.sandbox.try_into()
    }

    const fn output_file_name(self) -> &'static str {
        match self {
            Self::Frame => "thumbnail.png",
            Self::Probe | Self::AudioPresence | Self::Waveform => "analysis.json",
            _ => "output.media",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeSandboxEnvelopeV1 {
    max_source_bytes: u64,
    max_duration_ms: u64,
    max_width: u32,
    max_height: u32,
    max_decoded_bytes: u64,
    max_frames: u64,
    max_tracks: u16,
    max_memory_bytes: u64,
    max_scratch_bytes: u64,
    max_cpu_millis: u64,
    max_gpu_millis: u64,
    max_output_bytes: u64,
    max_cost_microunits: u64,
    network: NativeNetworkPolicyV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NativeNetworkPolicyV1 {
    Denied,
}

impl TryFrom<MediaSandboxLimits> for NativeSandboxEnvelopeV1 {
    type Error = NativeError;

    fn try_from(value: MediaSandboxLimits) -> Result<Self, Self::Error> {
        Ok(Self {
            max_source_bytes: value.max_source_bytes,
            max_duration_ms: value.max_duration_ms,
            max_width: value.max_width,
            max_height: value.max_height,
            max_decoded_bytes: value.max_decoded_bytes,
            max_frames: value.max_frames,
            max_tracks: value.max_tracks,
            max_memory_bytes: value.max_memory_bytes,
            max_scratch_bytes: value.max_scratch_bytes,
            max_cpu_millis: value.max_cpu_millis,
            max_gpu_millis: value.max_gpu_millis,
            max_output_bytes: value.max_output_bytes,
            max_cost_microunits: value.max_cost_microunits,
            network: match value.network {
                NetworkPolicy::Denied => NativeNetworkPolicyV1::Denied,
                NetworkPolicy::PrivateObjectBindingsOnly
                | NetworkPolicy::DeclaredProviderAdapterOnly => {
                    return Err(NativeError::InvalidPlan);
                }
            },
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeSourcePlanV1 {
    ordinal: u16,
    local_name: String,
    bytes: u64,
    checksum_sha256: String,
    content_type: String,
}

impl fmt::Debug for NativeSourcePlanV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSourcePlanV1")
            .field("ordinal", &self.ordinal)
            .field("local_name", &"<redacted>")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"<redacted>")
            .field("content_type", &self.content_type)
            .finish()
    }
}

impl NativeSourcePlanV1 {
    pub(crate) fn new(
        ordinal: u16,
        bytes: u64,
        checksum_sha256: String,
        content_type: String,
    ) -> Self {
        Self {
            ordinal,
            local_name: source_file_name(usize::from(ordinal)),
            bytes,
            checksum_sha256,
            content_type,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeExecutionPlanV1 {
    schema_version: u16,
    media_job_catalog_version: u16,
    media_service_catalog_version: u16,
    profile: NativeProfile,
    execution_origin: NativeExecutionOriginV1,
    sources: Vec<NativeSourcePlanV1>,
    output_local_name: String,
    output_content_type: String,
    output_max_bytes: u64,
    sandbox: NativeSandboxEnvelopeV1,
}

impl fmt::Debug for NativeExecutionPlanV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeExecutionPlanV1")
            .field("schema_version", &self.schema_version)
            .field("profile", &self.profile)
            .field("execution_origin", &self.execution_origin)
            .field("source_count", &self.sources.len())
            .field("output_content_type", &self.output_content_type)
            .field("output_max_bytes", &self.output_max_bytes)
            .finish()
    }
}

impl NativeExecutionPlanV1 {
    pub(crate) fn new(
        profile: NativeProfile,
        execution_origin: NativeExecutionOriginV1,
        sources: Vec<NativeSourcePlanV1>,
        output_content_type: String,
        output_max_bytes: u64,
    ) -> Result<Self, NativeError> {
        let spec = media_service_catalog()
            .get(profile.job_kind())
            .ok_or(NativeError::UnsupportedProfile)?;
        let plan = Self {
            schema_version: NATIVE_EXECUTION_PLAN_SCHEMA_VERSION,
            media_job_catalog_version: MEDIA_JOB_CATALOG_VERSION,
            media_service_catalog_version: MEDIA_SERVICE_CATALOG_VERSION,
            profile,
            execution_origin,
            sources,
            output_local_name: profile.output_file_name().to_owned(),
            output_content_type,
            output_max_bytes,
            sandbox: spec.sandbox.try_into()?,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub(crate) fn validate(&self) -> Result<(), NativeError> {
        let spec = media_service_catalog()
            .get(self.profile.job_kind())
            .ok_or(NativeError::UnsupportedProfile)?;
        let expected_sandbox = NativeSandboxEnvelopeV1::try_from(spec.sandbox)?;
        if self.schema_version != NATIVE_EXECUTION_PLAN_SCHEMA_VERSION
            || self.media_job_catalog_version != MEDIA_JOB_CATALOG_VERSION
            || self.media_service_catalog_version != MEDIA_SERVICE_CATALOG_VERSION
            || self.execution_origin != self.profile.expected_origin()
            || !self.profile.source_count().contains(&self.sources.len())
            || self.output_local_name != self.profile.output_file_name()
            || !spec
                .output_content_types
                .contains(&self.output_content_type.as_str())
            || self.output_max_bytes == 0
            || self.output_max_bytes > spec.sandbox.max_output_bytes
            || self.sandbox != expected_sandbox
        {
            return Err(NativeError::InvalidPlan);
        }
        let mut total_source_bytes = 0_u64;
        for (index, source) in self.sources.iter().enumerate() {
            if usize::from(source.ordinal) != index
                || source.local_name != source_file_name(index)
                || source.bytes == 0
                || source.bytes > spec.sandbox.max_source_bytes
                || !valid_sha256(&source.checksum_sha256)
                || !valid_source_content_type(&source.content_type)
            {
                return Err(NativeError::InvalidPlan);
            }
            total_source_bytes = total_source_bytes
                .checked_add(source.bytes)
                .ok_or(NativeError::ResourceLimit)?;
        }
        if total_source_bytes > spec.sandbox.max_source_bytes {
            return Err(NativeError::ResourceLimit);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn sources(&self) -> &[NativeSourcePlanV1] {
        &self.sources
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NativeOutput {
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) checksum_sha256: String,
    pub(crate) content_type: String,
}

impl fmt::Debug for NativeOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeOutput")
            .field("path", &"<redacted>")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"<redacted>")
            .field("content_type", &self.content_type)
            .finish()
    }
}

pub(crate) struct NativeAttempt {
    path: PathBuf,
}

impl NativeAttempt {
    pub(crate) fn create() -> Result<Self, NativeError> {
        let temporary_root =
            fs::canonicalize(env::temp_dir()).map_err(|_| NativeError::ResourceLimit)?;
        let path = temporary_root.join(format!(
            "frame-media-worker-job-{}",
            Uuid::now_v7().simple()
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|_| NativeError::ResourceLimit)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&path).map_err(|_| NativeError::ResourceLimit)?;
        Ok(Self { path })
    }

    pub(crate) fn source_path(&self, ordinal: usize) -> PathBuf {
        self.path.join(source_file_name(ordinal))
    }

    fn output_path(&self, plan: &NativeExecutionPlanV1) -> PathBuf {
        self.path.join(&plan.output_local_name)
    }

    fn plan_path(&self) -> PathBuf {
        self.path.join("native-plan-v1.json")
    }

    pub(crate) fn write_plan(&self, plan: &NativeExecutionPlanV1) -> Result<PathBuf, NativeError> {
        plan.validate()?;
        let bytes = serde_json::to_vec(plan).map_err(|_| NativeError::InvalidPlan)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_PLAN_BYTES {
            return Err(NativeError::InvalidPlan);
        }
        let path = self.plan_path();
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| NativeError::ResourceLimit)?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| NativeError::ResourceLimit)?;
        Ok(path)
    }
}

impl Drop for NativeAttempt {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeError {
    UnsupportedProfile,
    InvalidPlan,
    InvalidInput,
    MissingRuntime,
    CodecPolicyBlocked,
    UnsupportedGraph,
    Pipeline,
    Timeout,
    Cancelled,
    ResourceLimit,
    InvalidOutput,
}

impl NativeError {
    pub(crate) const fn error_class(self) -> &'static str {
        match self {
            Self::UnsupportedProfile | Self::CodecPolicyBlocked | Self::UnsupportedGraph => {
                "unsupported_media"
            }
            Self::InvalidPlan | Self::InvalidInput => "input_invalid",
            Self::MissingRuntime | Self::Pipeline => "pipeline_failure",
            Self::Timeout => "pipeline_timeout",
            Self::Cancelled => "cancelled",
            Self::ResourceLimit => "resource_limit",
            Self::InvalidOutput => "output_invalid",
        }
    }

    pub(crate) const fn retryable(self) -> bool {
        matches!(self, Self::MissingRuntime | Self::Timeout)
    }

    pub(crate) const fn child_exit_code(self) -> i32 {
        match self {
            Self::UnsupportedProfile => 40,
            Self::InvalidPlan => 41,
            Self::InvalidInput => 42,
            Self::MissingRuntime => 43,
            Self::CodecPolicyBlocked => 44,
            Self::UnsupportedGraph => 45,
            Self::Pipeline => 46,
            Self::Timeout => 47,
            Self::Cancelled => 48,
            Self::ResourceLimit => 49,
            Self::InvalidOutput => 50,
        }
    }

    const fn from_child_exit_code(code: i32) -> Option<Self> {
        match code {
            40 => Some(Self::UnsupportedProfile),
            41 => Some(Self::InvalidPlan),
            42 => Some(Self::InvalidInput),
            43 => Some(Self::MissingRuntime),
            44 => Some(Self::CodecPolicyBlocked),
            45 => Some(Self::UnsupportedGraph),
            46 => Some(Self::Pipeline),
            47 => Some(Self::Timeout),
            48 => Some(Self::Cancelled),
            49 | 72 => Some(Self::ResourceLimit),
            50 => Some(Self::InvalidOutput),
            _ => None,
        }
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedProfile => "the native media profile is unsupported",
            Self::InvalidPlan => "the native execution plan is invalid",
            Self::InvalidInput => "the native media input is invalid",
            Self::MissingRuntime => "the native media runtime is unavailable",
            Self::CodecPolicyBlocked => "the native codec policy is not approved",
            Self::UnsupportedGraph => "the native profile graph is unavailable",
            Self::Pipeline => "the native media pipeline failed",
            Self::Timeout => "the native media pipeline timed out",
            Self::Cancelled => "the native media pipeline was cancelled",
            Self::ResourceLimit => "the native media operation exceeded a resource limit",
            Self::InvalidOutput => "the native media output is invalid",
        })
    }
}

impl std::error::Error for NativeError {}

pub(crate) fn ensure_worker_runtime() -> Result<(), NativeError> {
    (native_sandbox_ready() && diagnose_runtime().is_ready())
        .then_some(())
        .ok_or(NativeError::MissingRuntime)
}

#[cfg(test)]
const fn native_sandbox_ready() -> bool {
    true
}

#[cfg(all(not(test), unix))]
fn native_sandbox_ready() -> bool {
    fs::metadata("/bin/sh").is_ok_and(|metadata| metadata.is_file())
        && fs::metadata("/bin/ps").is_ok_and(|metadata| metadata.is_file())
        && resident_set_bytes(std::process::id()).is_some()
}

#[cfg(all(not(test), not(unix)))]
const fn native_sandbox_ready() -> bool {
    false
}

pub(crate) fn ensure_profile_runtime(
    profile: NativeProfile,
    output_content_type: &str,
) -> Result<(), NativeError> {
    ensure_codec_policy(
        profile,
        output_content_type,
        env::var("FRAME_NATIVE_H264_AAC_APPROVED").ok().as_deref(),
    )?;
    ensure_worker_runtime()?;
    let required = required_factories(profile, output_content_type)?;
    let diagnostics = diagnose_runtime();
    let manifest = runtime_manifest();
    if required.iter().any(|required| {
        !manifest
            .factories
            .iter()
            .any(|declared| declared.factory == *required)
            || !diagnostics
                .factories
                .iter()
                .any(|factory| factory.factory == *required && factory.available)
    }) {
        return Err(NativeError::MissingRuntime);
    }
    Ok(())
}

fn ensure_codec_policy(
    profile: NativeProfile,
    output_content_type: &str,
    approval: Option<&str>,
) -> Result<(), NativeError> {
    if requires_h264_aac_approval(profile, output_content_type)
        && approval != Some(H264_AAC_APPROVAL)
    {
        return Err(NativeError::CodecPolicyBlocked);
    }
    Ok(())
}

fn required_factories(
    profile: NativeProfile,
    output_content_type: &str,
) -> Result<&'static [&'static str], NativeError> {
    let implementation = profile.implementation();
    if profile.has_implemented_graph()
        && implementation
            .implemented_output_content_types
            .contains(&output_content_type)
    {
        Ok(implementation.required_factories)
    } else {
        Err(NativeError::UnsupportedGraph)
    }
}

const fn requires_h264_aac_approval(profile: NativeProfile, output_content_type: &str) -> bool {
    matches!(output_content_type.as_bytes(), b"video/mp4" | b"audio/mp4")
        && !matches!(profile, NativeProfile::Frame)
}

pub(crate) fn run_native_plan(
    attempt: &NativeAttempt,
    plan: &NativeExecutionPlanV1,
    cancellation: &CancellationToken,
) -> Result<NativeOutput, NativeError> {
    if cancellation.is_cancelled() {
        return Err(NativeError::Cancelled);
    }
    plan.validate()?;
    ensure_profile_runtime(plan.profile, &plan.output_content_type)?;
    let plan_path = attempt.write_plan(plan)?;
    #[cfg(test)]
    {
        execute_child_plan(&plan_path, cancellation)?;
    }
    #[cfg(all(not(test), unix))]
    {
        run_isolated_child(&plan_path, plan, cancellation)?;
    }
    #[cfg(all(not(test), not(unix)))]
    {
        let _ = plan_path;
        return Err(NativeError::MissingRuntime);
    }
    let output = validate_output(attempt, plan, cancellation)?;
    if cancellation.is_cancelled() {
        return Err(NativeError::Cancelled);
    }
    Ok(output)
}

pub(crate) fn run_native_child(plan_path: &Path) -> Result<(), NativeError> {
    execute_child_plan(plan_path, &CancellationToken::new())
}

fn execute_child_plan(
    plan_path: &Path,
    cancellation: &CancellationToken,
) -> Result<(), NativeError> {
    let (attempt_path, plan) = read_and_validate_child_plan(plan_path)?;
    ensure_profile_runtime(plan.profile, &plan.output_content_type)?;
    validate_sources(&attempt_path, &plan)?;
    let output_path = attempt_path.join(&plan.output_local_name);
    if output_path.exists() {
        return Err(NativeError::InvalidOutput);
    }
    match plan.profile {
        NativeProfile::Frame => crate::thumbnail::run_thumbnail_child(
            &attempt_path.join(&plan.sources[0].local_name),
            &output_path,
            plan.output_max_bytes,
        )
        .map_err(map_thumbnail_error),
        NativeProfile::Probe | NativeProfile::AudioPresence | NativeProfile::Waveform => {
            run_analysis(
                &attempt_path.join(&plan.sources[0].local_name),
                &output_path,
                &plan,
                cancellation,
            )
        }
        _ => Err(NativeError::UnsupportedGraph),
    }
}

fn read_and_validate_child_plan(
    plan_path: &Path,
) -> Result<(PathBuf, NativeExecutionPlanV1), NativeError> {
    if plan_path.file_name().and_then(|value| value.to_str()) != Some("native-plan-v1.json") {
        return Err(NativeError::InvalidPlan);
    }
    let metadata = fs::symlink_metadata(plan_path).map_err(|_| NativeError::InvalidPlan)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_PLAN_BYTES
    {
        return Err(NativeError::InvalidPlan);
    }
    let canonical_plan = fs::canonicalize(plan_path).map_err(|_| NativeError::InvalidPlan)?;
    let attempt_path = canonical_plan
        .parent()
        .ok_or(NativeError::InvalidPlan)?
        .to_path_buf();
    validate_attempt_directory(&attempt_path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&canonical_plan)
        .and_then(|mut file| {
            Read::by_ref(&mut file)
                .take(MAX_PLAN_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| NativeError::InvalidPlan)?;
    let plan: NativeExecutionPlanV1 =
        serde_json::from_slice(&bytes).map_err(|_| NativeError::InvalidPlan)?;
    if serde_json::to_vec(&plan).map_err(|_| NativeError::InvalidPlan)? != bytes {
        return Err(NativeError::InvalidPlan);
    }
    plan.validate()?;
    Ok((attempt_path, plan))
}

fn validate_attempt_directory(path: &Path) -> Result<(), NativeError> {
    let temporary_root = fs::canonicalize(env::temp_dir()).map_err(|_| NativeError::InvalidPlan)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| NativeError::InvalidPlan)?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_prefix("frame-media-worker-job-"))
        .ok_or(NativeError::InvalidPlan)?;
    if !path.starts_with(temporary_root)
        || !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || name.len() != 32
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(NativeError::InvalidPlan);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(NativeError::InvalidPlan);
        }
    }
    Ok(())
}

fn validate_sources(path: &Path, plan: &NativeExecutionPlanV1) -> Result<(), NativeError> {
    for source in &plan.sources {
        let source_path = path.join(&source.local_name);
        let metadata = fs::symlink_metadata(&source_path).map_err(|_| NativeError::InvalidInput)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != source.bytes
            || sha256_file(&source_path)? != source.checksum_sha256
        {
            return Err(NativeError::InvalidInput);
        }
    }
    Ok(())
}

#[cfg(all(not(test), unix))]
fn run_isolated_child(
    plan_path: &Path,
    plan: &NativeExecutionPlanV1,
    cancellation: &CancellationToken,
) -> Result<(), NativeError> {
    if fs::metadata("/bin/sh").is_err() || fs::metadata("/bin/ps").is_err() {
        return Err(NativeError::MissingRuntime);
    }
    let executable = env::current_exe().map_err(|_| NativeError::MissingRuntime)?;
    let cpu_seconds = plan.sandbox.max_cpu_millis.div_ceil(1_000).max(1);
    // Shells disagree on whether `ulimit -f` units are 512 or 1,024 bytes.
    // Using the larger unit fails closed on both and still admits every catalog output.
    let file_units = plan.sandbox.max_scratch_bytes.div_ceil(1_024).max(1);
    let attempt_path = plan_path.parent().ok_or(NativeError::InvalidPlan)?;
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(
            "umask 077; ulimit -c 0 || exit 72; ulimit -t \"$1\" || exit 72; \
             ulimit -f \"$2\" || exit 72; exec \"$3\" native-child \"$4\"",
        )
        .arg("frame-native-sandbox")
        .arg(cpu_seconds.to_string())
        .arg(file_units.to_string())
        .arg(executable)
        .arg(plan_path)
        .env_clear()
        .current_dir(attempt_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in ["GST_PLUGIN_SYSTEM_PATH_1_0", "PATH", "TMPDIR"] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    if env::var("FRAME_NATIVE_H264_AAC_APPROVED").as_deref() == Ok(H264_AAC_APPROVAL) {
        command.env("FRAME_NATIVE_H264_AAC_APPROVED", H264_AAC_APPROVAL);
    }
    let mut child = command.spawn().map_err(|_| NativeError::MissingRuntime)?;
    let started = Instant::now();
    let wall_limit = Duration::from_millis(
        media_service_catalog()
            .get(plan.profile.job_kind())
            .ok_or(NativeError::InvalidPlan)?
            .timeout_ms,
    );
    loop {
        if cancellation.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NativeError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => return child_status_result(status),
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(NativeError::Pipeline);
            }
        }
        if started.elapsed() >= wall_limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NativeError::Timeout);
        }
        let rss = match resident_set_bytes(child.id()) {
            Some(rss) => rss,
            None => match child.try_wait() {
                Ok(Some(status)) => return child_status_result(status),
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(NativeError::ResourceLimit);
                }
            },
        };
        let scratch = match directory_bytes(attempt_path, plan.sandbox.max_scratch_bytes) {
            Ok(scratch) => scratch,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if rss > plan.sandbox.max_memory_bytes || scratch > plan.sandbox.max_scratch_bytes {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NativeError::ResourceLimit);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(all(not(test), unix))]
fn child_status_result(status: std::process::ExitStatus) -> Result<(), NativeError> {
    if status.success() {
        Ok(())
    } else {
        Err(status
            .code()
            .and_then(NativeError::from_child_exit_code)
            .unwrap_or(NativeError::Pipeline))
    }
}

#[cfg(all(not(test), unix))]
fn resident_set_bytes(process_id: u32) -> Option<u64> {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &process_id.to_string()])
        .env_clear()
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 64 {
        return None;
    }
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?
        .checked_mul(1_024)
}

#[cfg(all(not(test), unix))]
fn directory_bytes(path: &Path, limit: u64) -> Result<u64, NativeError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(|_| NativeError::ResourceLimit)? {
        let entry = entry.map_err(|_| NativeError::ResourceLimit)?;
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| NativeError::ResourceLimit)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(NativeError::ResourceLimit);
        }
        total = total
            .checked_add(metadata.len())
            .ok_or(NativeError::ResourceLimit)?;
        if total > limit {
            return Err(NativeError::ResourceLimit);
        }
    }
    Ok(total)
}

fn validate_output(
    attempt: &NativeAttempt,
    plan: &NativeExecutionPlanV1,
    cancellation: &CancellationToken,
) -> Result<NativeOutput, NativeError> {
    if cancellation.is_cancelled() {
        return Err(NativeError::Cancelled);
    }
    let path = attempt.output_path(plan);
    let metadata = fs::symlink_metadata(&path).map_err(|_| NativeError::InvalidOutput)?;
    let analysis_json = plan.output_content_type == "application/json";
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > plan.output_max_bytes
        || (analysis_json && metadata.len() > MAX_ANALYSIS_JSON_BYTES)
    {
        return Err(NativeError::InvalidOutput);
    }
    let mut prefix = [0_u8; 33];
    let mut file = File::open(&path).map_err(|_| NativeError::InvalidOutput)?;
    let read = file
        .read(&mut prefix)
        .map_err(|_| NativeError::InvalidOutput)?;
    match plan.output_content_type.as_str() {
        "image/png"
            if plan.profile == NativeProfile::Frame
                && read == prefix.len()
                && valid_thumbnail_png_prefix(&prefix) => {}
        "application/json" => {
            let bytes = fs::read(&path).map_err(|_| NativeError::InvalidOutput)?;
            validate_analysis_json_output(plan, &bytes)?;
        }
        _ => return Err(NativeError::InvalidOutput),
    }
    let checksum_sha256 = sha256_file_with_cancellation(&path, cancellation).map_err(|error| {
        if error == NativeError::Cancelled {
            error
        } else {
            NativeError::InvalidOutput
        }
    })?;
    Ok(NativeOutput {
        checksum_sha256,
        path,
        bytes: metadata.len(),
        content_type: plan.output_content_type.clone(),
    })
}

fn valid_thumbnail_png_prefix(prefix: &[u8; 33]) -> bool {
    prefix[..8] == *b"\x89PNG\r\n\x1a\n"
        && u32::from_be_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]) == 13
        && prefix[12..16] == *b"IHDR"
        && u32::from_be_bytes([prefix[16], prefix[17], prefix[18], prefix[19]]) == THUMBNAIL_WIDTH
        && u32::from_be_bytes([prefix[20], prefix[21], prefix[22], prefix[23]]) == THUMBNAIL_HEIGHT
}

fn validate_analysis_json_output(
    plan: &NativeExecutionPlanV1,
    bytes: &[u8],
) -> Result<(), NativeError> {
    match plan.profile {
        NativeProfile::Probe => {
            let manifest = canonical_json::<ProbeManifestV1>(bytes)?;
            let expected_container = probe_container(&plan.sources[0].content_type)
                .map_err(|_| NativeError::InvalidOutput)?
                .0;
            let video_codec_valid = matches!(
                manifest.video_codec.as_str(),
                "h264" | "h265" | "vp8" | "vp9" | "av1" | "prores" | "theora"
            );
            let audio_codec_valid = matches!(
                manifest.audio_codec.as_str(),
                "aac" | "mp3" | "opus" | "vorbis" | "flac" | "pcm" | "none"
            );
            let has_audio = manifest.audio_codec != "none";
            let minimum_tracks = if has_audio { 2 } else { 1 };
            if manifest.schema_version != NATIVE_EXECUTION_PLAN_SCHEMA_VERSION
                || manifest.profile != "probe_v1"
                || manifest.container != expected_container
                || !video_codec_valid
                || !audio_codec_valid
                || manifest.track_count < minimum_tracks
                || manifest.track_count > plan.sandbox.max_tracks
            {
                return Err(NativeError::InvalidOutput);
            }
            validate_analysis_bounds(
                &AnalysisState {
                    has_video: true,
                    has_audio,
                    width: Some(manifest.width),
                    height: Some(manifest.height),
                    frame_rate_numerator: Some(manifest.frame_rate_numerator),
                    frame_rate_denominator: Some(manifest.frame_rate_denominator),
                    sample_rate: has_audio.then_some(1),
                    channels: has_audio.then_some(1),
                    decoded_audio_bytes_per_second: u64::from(has_audio),
                    track_count: manifest.track_count,
                    ..AnalysisState::default()
                },
                manifest.duration_ms,
                plan,
            )
            .map_err(|_| NativeError::InvalidOutput)
        }
        NativeProfile::AudioPresence => {
            let manifest = canonical_json::<AudioPresenceManifestV1>(bytes)?;
            if manifest.schema_version != NATIVE_EXECUTION_PLAN_SCHEMA_VERSION
                || manifest.profile != "audio_presence_v1"
                || manifest.track_count == 0
                || manifest.track_count > plan.sandbox.max_tracks
            {
                return Err(NativeError::InvalidOutput);
            }
            Ok(())
        }
        NativeProfile::Waveform => {
            let manifest = canonical_json::<WaveformManifestV1>(bytes)?;
            if manifest.schema_version != NATIVE_EXECUTION_PLAN_SCHEMA_VERSION
                || manifest.profile != "waveform_v1"
                || manifest.duration_ms == 0
                || manifest.duration_ms > plan.sandbox.max_duration_ms
                || !(1..=MAX_AUDIO_SAMPLE_RATE).contains(&u64::from(manifest.sample_rate))
                || !(1..=MAX_AUDIO_CHANNELS).contains(&u64::from(manifest.channels))
                || manifest.waveform_milli.is_empty()
                || manifest.waveform_milli.len() > MAX_WAVEFORM_POINTS
                || manifest.waveform_milli.iter().any(|value| *value > 1_000)
            {
                return Err(NativeError::InvalidOutput);
            }
            let decoded_bytes = u128::from(manifest.duration_ms)
                .checked_mul(u128::from(manifest.sample_rate))
                .and_then(|value| value.checked_mul(u128::from(manifest.channels)))
                .and_then(|value| value.checked_mul(u128::from(MAX_AUDIO_SAMPLE_BYTES)))
                .and_then(|value| value.checked_add(999))
                .and_then(|value| value.checked_div(1_000))
                .filter(|value| *value <= u128::from(plan.sandbox.max_decoded_bytes))
                .ok_or(NativeError::InvalidOutput)?;
            if decoded_bytes == 0 {
                return Err(NativeError::InvalidOutput);
            }
            Ok(())
        }
        _ => Err(NativeError::InvalidOutput),
    }
}

fn canonical_json<T>(bytes: &[u8]) -> Result<T, NativeError>
where
    T: DeserializeOwned + Serialize,
{
    let value = serde_json::from_slice::<T>(bytes).map_err(|_| NativeError::InvalidOutput)?;
    if serde_json::to_vec(&value).map_err(|_| NativeError::InvalidOutput)? != bytes {
        return Err(NativeError::InvalidOutput);
    }
    Ok(value)
}

#[derive(Debug, Clone, Default)]
struct AnalysisState {
    has_video: bool,
    has_audio: bool,
    width: Option<u32>,
    height: Option<u32>,
    frame_rate_numerator: Option<u32>,
    frame_rate_denominator: Option<u32>,
    sample_rate: Option<u32>,
    channels: Option<u32>,
    decoded_audio_bytes_per_second: u64,
    track_count: u16,
    observed_container: Option<&'static str>,
    video_codec: Option<&'static str>,
    audio_codec: Option<&'static str>,
    waveform: WaveformAccumulator,
    branch_failure: bool,
}

#[derive(Debug, Clone, Default)]
struct WaveformAccumulator {
    points: Vec<u16>,
}

impl WaveformAccumulator {
    fn push_buffer(&mut self, bytes: &[u8]) {
        let mut sum = 0.0_f64;
        let mut count = 0_u64;
        for chunk in bytes.chunks_exact(4) {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if sample.is_finite() {
                sum += f64::from(sample.abs());
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        let normalized = ((sum / count as f64) * 1_000.0).round().clamp(0.0, 1_000.0) as u16;
        self.points.push(normalized);
        if self.points.len() > MAX_WAVEFORM_POINTS {
            self.points = self
                .points
                .chunks(2)
                .map(|pair| {
                    let total = pair.iter().copied().map(u32::from).sum::<u32>();
                    u16::try_from(total / pair.len() as u32).unwrap_or(1_000)
                })
                .collect();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeManifestV1 {
    schema_version: u16,
    profile: String,
    container: String,
    video_codec: String,
    audio_codec: String,
    duration_ms: u64,
    width: u32,
    height: u32,
    frame_rate_numerator: u32,
    frame_rate_denominator: u32,
    track_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioPresenceManifestV1 {
    schema_version: u16,
    profile: String,
    has_audio: bool,
    track_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaveformManifestV1 {
    schema_version: u16,
    profile: String,
    duration_ms: u64,
    sample_rate: u32,
    channels: u32,
    waveform_milli: Vec<u16>,
}

fn run_analysis(
    source_path: &Path,
    output_path: &Path,
    plan: &NativeExecutionPlanV1,
    cancellation: &CancellationToken,
) -> Result<(), NativeError> {
    let pipeline = gst::parse::launch("filesrc name=source ! decodebin name=decode")
        .map_err(|_| NativeError::MissingRuntime)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| NativeError::Pipeline)?;
    let source = pipeline.by_name("source").ok_or(NativeError::Pipeline)?;
    source.set_property("location", source_path);
    let decode = pipeline.by_name("decode").ok_or(NativeError::Pipeline)?;
    let state = Arc::new(Mutex::new(AnalysisState::default()));
    let encoded_state = Arc::clone(&state);
    decode.connect("autoplug-continue", false, move |values| {
        if let Some(caps) = values
            .get(2)
            .and_then(|value| value.get::<gst::Caps>().ok())
            && let Ok(mut locked) = encoded_state.lock()
        {
            observe_encoded_caps(&caps, &mut locked);
        }
        Some(true.to_value())
    });
    let weak_pipeline = pipeline.downgrade();
    let branch_state = Arc::clone(&state);
    let waveform = plan.profile == NativeProfile::Waveform;
    let max_tracks = plan.sandbox.max_tracks;
    decode.connect_pad_added(move |_decode, pad| {
        let Some(pipeline) = weak_pipeline.upgrade() else {
            return;
        };
        let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
        let Some(structure) = caps.structure(0) else {
            mark_branch_failure(&branch_state);
            return;
        };
        let media_type = structure.name();
        let admitted = if let Ok(mut locked) = branch_state.lock() {
            increment_track_count(&mut locked);
            if locked.track_count > max_tracks {
                locked.branch_failure = true;
                false
            } else {
                true
            }
        } else {
            false
        };
        if !admitted {
            return;
        }
        if media_type.starts_with("video/") {
            if let Ok(mut locked) = branch_state.lock() {
                if locked.has_video {
                    locked.branch_failure = true;
                } else {
                    locked.has_video = true;
                    locked.width = positive_u32(structure, "width");
                    locked.height = positive_u32(structure, "height");
                    if let Ok(rate) = structure.get::<gst::Fraction>("framerate") {
                        locked.frame_rate_numerator = u32::try_from(rate.numer()).ok();
                        locked.frame_rate_denominator = u32::try_from(rate.denom()).ok();
                    }
                }
            }
            if add_discard_branch(&pipeline, pad).is_err() {
                mark_branch_failure(&branch_state);
            }
        } else if media_type.starts_with("audio/") {
            let mut collect_waveform = false;
            if let Ok(mut locked) = branch_state.lock() {
                let sample_rate = positive_u32(structure, "rate");
                let channels = positive_u32(structure, "channels");
                let decoded_bytes_per_second =
                    sample_rate
                        .zip(channels)
                        .and_then(|(sample_rate, channels)| {
                            u64::from(sample_rate)
                                .checked_mul(u64::from(channels))
                                .and_then(|value| value.checked_mul(MAX_AUDIO_SAMPLE_BYTES))
                        });
                let Some(decoded_bytes_per_second) = decoded_bytes_per_second else {
                    locked.branch_failure = true;
                    return;
                };
                let Some(total) = locked
                    .decoded_audio_bytes_per_second
                    .checked_add(decoded_bytes_per_second)
                else {
                    locked.branch_failure = true;
                    return;
                };
                locked.decoded_audio_bytes_per_second = total;
                if !locked.has_audio {
                    locked.has_audio = true;
                    collect_waveform = waveform;
                    locked.sample_rate = sample_rate;
                    locked.channels = channels;
                }
            }
            let result = if collect_waveform {
                add_waveform_branch(&pipeline, pad, Arc::clone(&branch_state))
            } else {
                add_discard_branch(&pipeline, pad)
            };
            if result.is_err() {
                mark_branch_failure(&branch_state);
            }
        } else if add_discard_branch(&pipeline, pad).is_err() {
            mark_branch_failure(&branch_state);
        }
    });
    if !pipeline_has_declared_top_level_factories(&pipeline) {
        return Err(NativeError::MissingRuntime);
    }
    let bus = pipeline.bus().ok_or(NativeError::Pipeline)?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| NativeError::InvalidInput)?;
    let started = Instant::now();
    let timeout = media_service_catalog()
        .get(plan.profile.job_kind())
        .map(|spec| Duration::from_millis(spec.timeout_ms))
        .ok_or(NativeError::InvalidPlan)?;
    let terminal = loop {
        if cancellation.is_cancelled() {
            break Err(NativeError::Cancelled);
        }
        if started.elapsed() >= timeout {
            break Err(NativeError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL.as_millis() as u64),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            gst::MessageView::Error(_) => break Err(NativeError::InvalidInput),
            _ => {}
        }
    };
    let duration_ms = pipeline
        .query_duration::<gst::ClockTime>()
        .map(|value| value.mseconds())
        .filter(|value| *value > 0)
        .ok_or(NativeError::InvalidInput);
    let trusted = pipeline_has_trusted_factory_provenance(&pipeline);
    let teardown = pipeline.set_state(gst::State::Null);
    terminal?;
    let duration_ms = duration_ms?;
    if !trusted || teardown.is_err() {
        return Err(NativeError::MissingRuntime);
    }
    let state = state.lock().map_err(|_| NativeError::Pipeline)?.clone();
    if state.branch_failure
        || (!state.has_video && !state.has_audio)
        || state.track_count == 0
        || state.track_count > plan.sandbox.max_tracks
        || (plan.profile == NativeProfile::Waveform
            && (!state.has_audio || state.waveform.points.is_empty()))
    {
        return Err(NativeError::InvalidInput);
    }
    validate_analysis_bounds(&state, duration_ms, plan)?;
    match plan.profile {
        NativeProfile::Probe => {
            let (container, observed_container) = probe_container(&plan.sources[0].content_type)?;
            if !state.has_video || state.observed_container != Some(observed_container) {
                return Err(NativeError::InvalidInput);
            }
            write_json_output(
                output_path,
                plan.output_max_bytes,
                &ProbeManifestV1 {
                    schema_version: NATIVE_EXECUTION_PLAN_SCHEMA_VERSION,
                    profile: "probe_v1".into(),
                    container: container.into(),
                    video_codec: state.video_codec.ok_or(NativeError::InvalidInput)?.into(),
                    audio_codec: if state.has_audio {
                        state.audio_codec.ok_or(NativeError::InvalidInput)?
                    } else {
                        "none"
                    }
                    .into(),
                    duration_ms,
                    width: state.width.ok_or(NativeError::InvalidInput)?,
                    height: state.height.ok_or(NativeError::InvalidInput)?,
                    frame_rate_numerator: state
                        .frame_rate_numerator
                        .filter(|value| *value > 0)
                        .ok_or(NativeError::InvalidInput)?,
                    frame_rate_denominator: state
                        .frame_rate_denominator
                        .filter(|value| *value > 0)
                        .ok_or(NativeError::InvalidInput)?,
                    track_count: state.track_count,
                },
            )
        }
        NativeProfile::AudioPresence => write_json_output(
            output_path,
            plan.output_max_bytes,
            &AudioPresenceManifestV1 {
                schema_version: NATIVE_EXECUTION_PLAN_SCHEMA_VERSION,
                profile: "audio_presence_v1".into(),
                has_audio: state.has_audio,
                track_count: state.track_count,
            },
        ),
        NativeProfile::Waveform => write_json_output(
            output_path,
            plan.output_max_bytes,
            &WaveformManifestV1 {
                schema_version: NATIVE_EXECUTION_PLAN_SCHEMA_VERSION,
                profile: "waveform_v1".into(),
                duration_ms,
                sample_rate: state.sample_rate.ok_or(NativeError::InvalidInput)?,
                channels: state.channels.ok_or(NativeError::InvalidInput)?,
                waveform_milli: state.waveform.points,
            },
        ),
        _ => Err(NativeError::UnsupportedGraph),
    }
}

fn increment_track_count(state: &mut AnalysisState) {
    if let Some(track_count) = state.track_count.checked_add(1) {
        state.track_count = track_count;
    } else {
        state.branch_failure = true;
    }
}

fn validate_analysis_bounds(
    state: &AnalysisState,
    duration_ms: u64,
    plan: &NativeExecutionPlanV1,
) -> Result<(), NativeError> {
    if duration_ms == 0 {
        return Err(NativeError::InvalidInput);
    }
    if duration_ms > plan.sandbox.max_duration_ms {
        return Err(NativeError::ResourceLimit);
    }

    let mut decoded_bytes = 0_u128;
    if state.has_video {
        let width = state.width.ok_or(NativeError::InvalidInput)?;
        let height = state.height.ok_or(NativeError::InvalidInput)?;
        let frame_rate_numerator = state
            .frame_rate_numerator
            .filter(|value| *value > 0)
            .ok_or(NativeError::InvalidInput)?;
        let frame_rate_denominator = state
            .frame_rate_denominator
            .filter(|value| *value > 0)
            .ok_or(NativeError::InvalidInput)?;
        if width > plan.sandbox.max_width
            || height > plan.sandbox.max_height
            || u64::from(frame_rate_numerator)
                > u64::from(frame_rate_denominator) * MAX_PROBE_FRAME_RATE
        {
            return Err(NativeError::ResourceLimit);
        }
        let frame_denominator = 1_000_u128 * u128::from(frame_rate_denominator);
        let frame_numerator = u128::from(duration_ms) * u128::from(frame_rate_numerator);
        let frames = frame_numerator
            .checked_add(frame_denominator - 1)
            .and_then(|value| value.checked_div(frame_denominator))
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or(NativeError::ResourceLimit)?;
        if frames > plan.sandbox.max_frames {
            return Err(NativeError::ResourceLimit);
        }
        decoded_bytes = u128::from(frames)
            .checked_mul(u128::from(width))
            .and_then(|value| value.checked_mul(u128::from(height)))
            .and_then(|value| value.checked_mul(4))
            .ok_or(NativeError::ResourceLimit)?;
    }

    if state.has_audio {
        let sample_rate = state.sample_rate.ok_or(NativeError::InvalidInput)?;
        let channels = state.channels.ok_or(NativeError::InvalidInput)?;
        if sample_rate == 0 || channels == 0 {
            return Err(NativeError::InvalidInput);
        }
        if u64::from(sample_rate) > MAX_AUDIO_SAMPLE_RATE
            || u64::from(channels) > MAX_AUDIO_CHANNELS
        {
            return Err(NativeError::ResourceLimit);
        }
        let audio_bytes_per_second = if plan.profile == NativeProfile::Probe {
            MAX_AUDIO_SAMPLE_RATE
                .checked_mul(MAX_AUDIO_CHANNELS)
                .and_then(|value| value.checked_mul(MAX_AUDIO_SAMPLE_BYTES))
                .ok_or(NativeError::ResourceLimit)?
        } else {
            state.decoded_audio_bytes_per_second
        };
        if audio_bytes_per_second == 0 {
            return Err(NativeError::InvalidInput);
        }
        let audio_bytes = u128::from(duration_ms)
            .checked_mul(u128::from(audio_bytes_per_second))
            .and_then(|value| value.checked_add(999))
            .and_then(|value| value.checked_div(1_000))
            .ok_or(NativeError::ResourceLimit)?;
        decoded_bytes = decoded_bytes
            .checked_add(audio_bytes)
            .ok_or(NativeError::ResourceLimit)?;
    }

    if decoded_bytes == 0 || decoded_bytes > u128::from(plan.sandbox.max_decoded_bytes) {
        return Err(NativeError::ResourceLimit);
    }
    Ok(())
}

fn observe_encoded_caps(caps: &gst::Caps, state: &mut AnalysisState) {
    for structure in caps.iter() {
        let name = structure.name();
        if let Some(container) = match name.as_str() {
            "video/quicktime" => Some("quicktime-family"),
            "video/webm" => Some("webm"),
            "video/x-matroska" => Some("matroska"),
            _ => None,
        } {
            record_label(
                &mut state.observed_container,
                container,
                &mut state.branch_failure,
            );
        }
        if let Some(codec) = match name.as_str() {
            "video/x-h264" => Some("h264"),
            "video/x-h265" => Some("h265"),
            "video/x-vp8" => Some("vp8"),
            "video/x-vp9" => Some("vp9"),
            "video/x-av1" => Some("av1"),
            "video/x-prores" => Some("prores"),
            "video/x-theora" => Some("theora"),
            _ => None,
        } {
            record_label(&mut state.video_codec, codec, &mut state.branch_failure);
        }
        if let Some(codec) = match name.as_str() {
            "audio/x-opus" => Some("opus"),
            "audio/x-vorbis" => Some("vorbis"),
            "audio/x-flac" => Some("flac"),
            "audio/x-alaw" | "audio/x-mulaw" => Some("pcm"),
            "audio/mpeg" if structure.get::<i32>("mpegversion") == Ok(4) => Some("aac"),
            "audio/mpeg"
                if structure.get::<i32>("mpegversion") == Ok(1)
                    && structure.get::<i32>("layer") == Ok(3) =>
            {
                Some("mp3")
            }
            _ => None,
        } {
            record_label(&mut state.audio_codec, codec, &mut state.branch_failure);
        }
    }
}

fn record_label(slot: &mut Option<&'static str>, value: &'static str, branch_failure: &mut bool) {
    if slot.is_some_and(|current| current != value) {
        *branch_failure = true;
    } else {
        *slot = Some(value);
    }
}

fn positive_u32(structure: &gst::StructureRef, field: &str) -> Option<u32> {
    structure
        .get::<i32>(field)
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn probe_container(content_type: &str) -> Result<(&'static str, &'static str), NativeError> {
    match content_type {
        "video/mp4" => Ok(("mp4", "quicktime-family")),
        "video/quicktime" => Ok(("quicktime", "quicktime-family")),
        "video/webm" => Ok(("webm", "webm")),
        "video/x-matroska" => Ok(("matroska", "matroska")),
        _ => Err(NativeError::InvalidInput),
    }
}

fn pipeline_has_declared_top_level_factories(pipeline: &gst::Pipeline) -> bool {
    let manifest = runtime_manifest();
    pipeline.children().iter().all(|element| {
        element.factory().is_some_and(|factory| {
            manifest
                .factories
                .iter()
                .any(|declared| declared.factory == factory.name().as_str())
        })
    })
}

fn make(factory: &'static str) -> Result<gst::Element, NativeError> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| NativeError::MissingRuntime)
}

fn add_discard_branch(pipeline: &gst::Pipeline, pad: &gst::Pad) -> Result<(), NativeError> {
    let queue = make("queue")?;
    let sink = make("fakesink")?;
    sink.set_property("sync", false);
    pipeline
        .add_many([&queue, &sink])
        .map_err(|_| NativeError::Pipeline)?;
    queue.link(&sink).map_err(|_| NativeError::Pipeline)?;
    queue
        .sync_state_with_parent()
        .map_err(|_| NativeError::Pipeline)?;
    sink.sync_state_with_parent()
        .map_err(|_| NativeError::Pipeline)?;
    let sink_pad = queue.static_pad("sink").ok_or(NativeError::Pipeline)?;
    pad.link(&sink_pad).map_err(|_| NativeError::InvalidInput)?;
    Ok(())
}

fn add_waveform_branch(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    state: Arc<Mutex<AnalysisState>>,
) -> Result<(), NativeError> {
    let queue = make("queue")?;
    let convert = make("audioconvert")?;
    let resample = make("audioresample")?;
    let capsfilter = make("capsfilter")?;
    capsfilter.set_property(
        "caps",
        gst::Caps::builder("audio/x-raw")
            .field("format", "F32LE")
            .field("rate", 8_000_i32)
            .field("channels", 1_i32)
            .build(),
    );
    let identity = make("identity")?;
    let sink = make("fakesink")?;
    sink.set_property("sync", false);
    pipeline
        .add_many([&queue, &convert, &resample, &capsfilter, &identity, &sink])
        .map_err(|_| NativeError::Pipeline)?;
    gst::Element::link_many([&queue, &convert, &resample, &capsfilter, &identity, &sink])
        .map_err(|_| NativeError::Pipeline)?;
    let probe_pad = identity.static_pad("src").ok_or(NativeError::Pipeline)?;
    probe_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
        if let Some(gst::PadProbeData::Buffer(buffer)) = info.data.as_ref()
            && let Ok(map) = buffer.map_readable()
            && let Ok(mut locked) = state.lock()
        {
            locked.waveform.push_buffer(map.as_slice());
        }
        gst::PadProbeReturn::Ok
    });
    for element in [&queue, &convert, &resample, &capsfilter, &identity, &sink] {
        element
            .sync_state_with_parent()
            .map_err(|_| NativeError::Pipeline)?;
    }
    let sink_pad = queue.static_pad("sink").ok_or(NativeError::Pipeline)?;
    pad.link(&sink_pad).map_err(|_| NativeError::InvalidInput)?;
    Ok(())
}

fn mark_branch_failure(state: &Arc<Mutex<AnalysisState>>) {
    if let Ok(mut locked) = state.lock() {
        locked.branch_failure = true;
    }
}

fn write_json_output<T: Serialize>(path: &Path, limit: u64, value: &T) -> Result<(), NativeError> {
    let bytes = serde_json::to_vec(value).map_err(|_| NativeError::InvalidOutput)?;
    if bytes.is_empty() || bytes.len() as u64 > limit.min(MAX_ANALYSIS_JSON_BYTES) {
        return Err(NativeError::ResourceLimit);
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| NativeError::InvalidOutput)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| NativeError::InvalidOutput)
}

fn sha256_file(path: &Path) -> Result<String, NativeError> {
    sha256_file_inner(path, None)
}

fn sha256_file_with_cancellation(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<String, NativeError> {
    sha256_file_inner(path, Some(cancellation))
}

fn sha256_file_inner(
    path: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<String, NativeError> {
    let mut file = File::open(path).map_err(|_| NativeError::InvalidInput)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    loop {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(NativeError::Cancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|_| NativeError::InvalidInput)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn source_file_name(ordinal: usize) -> String {
    if ordinal == 0 {
        "source.media".to_owned()
    } else {
        format!("source-{ordinal:03}.media")
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_source_content_type(value: &str) -> bool {
    matches!(
        value,
        "video/mp4"
            | "video/quicktime"
            | "video/webm"
            | "video/x-matroska"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/wav"
            | "audio/webm"
            | "audio/ogg"
            | "application/json"
    )
}

fn map_thumbnail_error(error: crate::thumbnail::ThumbnailError) -> NativeError {
    match error {
        crate::thumbnail::ThumbnailError::InvalidInput => NativeError::InvalidInput,
        crate::thumbnail::ThumbnailError::MissingRuntime => NativeError::MissingRuntime,
        crate::thumbnail::ThumbnailError::Pipeline => NativeError::Pipeline,
        crate::thumbnail::ThumbnailError::Timeout => NativeError::Timeout,
        crate::thumbnail::ThumbnailError::Cancelled => NativeError::Cancelled,
        crate::thumbnail::ThumbnailError::ResourceLimit => NativeError::ResourceLimit,
        crate::thumbnail::ThumbnailError::InvalidOutput => NativeError::InvalidOutput,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(path: &Path) -> String {
        sha256_file(path).expect("checksum")
    }

    fn analysis_plan(
        profile: NativeProfile,
        bytes: u64,
        checksum: String,
    ) -> NativeExecutionPlanV1 {
        NativeExecutionPlanV1::new(
            profile,
            profile.expected_origin(),
            vec![NativeSourcePlanV1::new(
                0,
                bytes,
                checksum,
                "video/webm".into(),
            )],
            "application/json".into(),
            1_000_000,
        )
        .expect("plan")
    }

    #[test]
    fn retained_native_catalog_has_a_closed_versioned_plan_contract() {
        assert_eq!(NativeProfile::ALL.len(), 14);
        for profile in NativeProfile::ALL {
            assert_eq!(NativeProfile::parse(profile.profile_id()), Ok(profile));
            let encoded = serde_json::to_string(&profile).expect("profile serialization");
            assert_eq!(encoded, format!("\"{}\"", profile.profile_id()));
            let decoded =
                serde_json::from_str::<NativeProfile>(&encoded).expect("profile deserialization");
            assert_eq!(decoded, profile);
            let spec = media_service_catalog()
                .get(profile.job_kind())
                .expect("catalog row");
            assert_ne!(
                spec.disposition,
                MediaExecutionDisposition::ExternalProviderAdapter
            );
            let expected_origin = match spec.disposition {
                MediaExecutionDisposition::HybridManagedNative => {
                    NativeExecutionOriginV1::ManagedFallback
                }
                MediaExecutionDisposition::NativeOnly => NativeExecutionOriginV1::NativeOnly,
                MediaExecutionDisposition::ExternalProviderAdapter => unreachable!(),
            };
            assert_eq!(profile.expected_origin(), expected_origin);
        }
        assert_eq!(
            NativeProfile::parse("transcription_v1"),
            Err(NativeError::UnsupportedProfile)
        );
        assert_eq!(
            NativeProfile::parse("ai_cleanup_v1"),
            Err(NativeError::UnsupportedProfile)
        );
    }

    #[test]
    fn every_native_profile_has_an_audited_graph_or_typed_exception() {
        use std::collections::HashSet;

        let declared_factories: HashSet<_> = runtime_manifest()
            .factories
            .iter()
            .map(|factory| factory.factory)
            .collect();
        let mut graph_ids = HashSet::new();
        let mut executable = 0;
        let mut exceptions = 0;

        for profile in NativeProfile::ALL {
            let implementation = profile.implementation();
            assert!(
                implementation
                    .graph_id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            );
            assert!(graph_ids.insert(implementation.graph_id));
            assert!(!implementation.required_factories.is_empty());
            assert!(
                implementation
                    .required_factories
                    .iter()
                    .all(|factory| declared_factories.contains(factory)),
                "{profile:?} references a factory outside the pinned runtime manifest"
            );
            match implementation.state {
                NativeImplementationStateV1::Executable => {
                    executable += 1;
                    assert!(!implementation.implemented_output_content_types.is_empty());
                    assert_eq!(implementation.exception, None);
                }
                NativeImplementationStateV1::ExecutableWithVariantException => {
                    executable += 1;
                    exceptions += 1;
                    assert!(!implementation.implemented_output_content_types.is_empty());
                    assert!(implementation.exception.is_some());
                }
                NativeImplementationStateV1::DocumentedException => {
                    exceptions += 1;
                    assert!(implementation.implemented_output_content_types.is_empty());
                    assert!(implementation.exception.is_some());
                }
            }
            assert_eq!(
                profile.has_implemented_graph(),
                implementation.state != NativeImplementationStateV1::DocumentedException
            );
        }

        assert_eq!(graph_ids.len(), NativeProfile::ALL.len());
        assert_eq!(executable, 4);
        assert_eq!(exceptions, 11);
    }

    #[test]
    fn child_exit_protocol_preserves_every_privacy_safe_failure_class() {
        for error in [
            NativeError::UnsupportedProfile,
            NativeError::InvalidPlan,
            NativeError::InvalidInput,
            NativeError::MissingRuntime,
            NativeError::CodecPolicyBlocked,
            NativeError::UnsupportedGraph,
            NativeError::Pipeline,
            NativeError::Timeout,
            NativeError::Cancelled,
            NativeError::ResourceLimit,
            NativeError::InvalidOutput,
        ] {
            assert_eq!(
                NativeError::from_child_exit_code(error.child_exit_code()),
                Some(error)
            );
        }
        assert_eq!(NativeError::from_child_exit_code(1), None);
    }

    #[test]
    fn native_debug_contract_redacts_paths_and_checksums() {
        let private_path = "/private/tenant/source/output.png";
        let private_checksum = "a".repeat(64);
        let output = NativeOutput {
            path: private_path.into(),
            bytes: 12,
            checksum_sha256: private_checksum.clone(),
            content_type: "image/png".into(),
        };
        let debug = format!("{output:?}");
        assert!(!debug.contains(private_path));
        assert!(!debug.contains(&private_checksum));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn multi_source_profiles_require_canonical_dense_ordinals() {
        let source =
            |ordinal| NativeSourcePlanV1::new(ordinal, 10, "1".repeat(64), "video/webm".into());
        let plan = NativeExecutionPlanV1::new(
            NativeProfile::SegmentMux,
            NativeExecutionOriginV1::NativeOnly,
            vec![source(0), source(1)],
            "video/mp4".into(),
            1_000_000,
        )
        .expect("multi-source plan");
        assert_eq!(plan.sources().len(), 2);
        let serialized = serde_json::to_value(&plan).expect("serialized plan");
        assert_eq!(serialized["profile"], "segment_mux_v1");
        let mut forged = plan;
        forged.sources[1].ordinal = 7;
        assert_eq!(forged.validate(), Err(NativeError::InvalidPlan));
    }

    #[test]
    fn h264_aac_and_untrusted_graphs_never_silently_claim_support() {
        assert_eq!(
            ensure_codec_policy(NativeProfile::DistributionMaster, "video/mp4", None),
            Err(NativeError::CodecPolicyBlocked)
        );
        assert_eq!(
            ensure_codec_policy(
                NativeProfile::DistributionMaster,
                "video/mp4",
                Some(H264_AAC_APPROVAL)
            ),
            Ok(())
        );
        assert_eq!(
            required_factories(NativeProfile::Spritesheet, "image/jpeg"),
            Err(NativeError::UnsupportedGraph)
        );
        assert_eq!(
            required_factories(NativeProfile::SegmentMux, "video/mp4"),
            Err(NativeError::UnsupportedGraph)
        );
        assert!(!requires_h264_aac_approval(
            NativeProfile::AnimatedPreview,
            "image/gif"
        ));
    }

    #[test]
    fn analysis_bounds_enforce_observed_geometry_rate_frames_and_decoded_bytes() {
        let plan = analysis_plan(NativeProfile::Probe, 10, "1".repeat(64));
        let valid = AnalysisState {
            has_video: true,
            has_audio: false,
            width: Some(320),
            height: Some(180),
            frame_rate_numerator: Some(30),
            frame_rate_denominator: Some(1),
            track_count: 1,
            ..AnalysisState::default()
        };
        assert_eq!(validate_analysis_bounds(&valid, 2_000, &plan), Ok(()));

        let mut oversized = valid.clone();
        oversized.width = Some(plan.sandbox.max_width + 1);
        assert_eq!(
            validate_analysis_bounds(&oversized, 2_000, &plan),
            Err(NativeError::ResourceLimit)
        );

        let mut excessive_rate = valid.clone();
        excessive_rate.frame_rate_numerator = Some(241);
        assert_eq!(
            validate_analysis_bounds(&excessive_rate, 2_000, &plan),
            Err(NativeError::ResourceLimit)
        );

        let mut tiny_envelope = plan;
        tiny_envelope.sandbox.max_decoded_bytes = 1;
        assert_eq!(
            validate_analysis_bounds(&valid, 2_000, &tiny_envelope),
            Err(NativeError::ResourceLimit)
        );
    }

    #[test]
    fn parent_accepts_only_profile_exact_canonical_analysis_json() {
        let attempt = NativeAttempt::create().expect("attempt");
        let plan = analysis_plan(NativeProfile::Probe, 10, "1".repeat(64));
        let manifest = ProbeManifestV1 {
            schema_version: 1,
            profile: "probe_v1".into(),
            container: "webm".into(),
            video_codec: "vp8".into(),
            audio_codec: "none".into(),
            duration_ms: 2_000,
            width: 320,
            height: 180,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            track_count: 1,
        };
        let canonical = serde_json::to_vec(&manifest).expect("manifest");
        fs::write(attempt.output_path(&plan), &canonical).expect("canonical output");
        assert!(validate_output(&attempt, &plan, &CancellationToken::new()).is_ok());

        fs::remove_file(attempt.output_path(&plan)).expect("remove output");
        let mut noncanonical = vec![b' '];
        noncanonical.extend_from_slice(&canonical);
        fs::write(attempt.output_path(&plan), noncanonical).expect("noncanonical output");
        assert_eq!(
            validate_output(&attempt, &plan, &CancellationToken::new()),
            Err(NativeError::InvalidOutput)
        );
    }

    #[test]
    fn child_accepts_only_canonical_plan_json() {
        let attempt = NativeAttempt::create().expect("attempt");
        let plan = analysis_plan(NativeProfile::Probe, 10, "1".repeat(64));
        let mut noncanonical = vec![b' '];
        noncanonical.extend_from_slice(&serde_json::to_vec(&plan).expect("plan"));
        fs::write(attempt.plan_path(), noncanonical).expect("plan file");
        assert_eq!(
            read_and_validate_child_plan(&attempt.plan_path()),
            Err(NativeError::InvalidPlan)
        );
    }

    #[cfg(unix)]
    #[test]
    fn attempt_directory_is_private_at_creation() {
        use std::os::unix::fs::PermissionsExt;

        let attempt = NativeAttempt::create().expect("attempt");
        let mode = fs::metadata(&attempt.path)
            .expect("attempt metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
        let canonical = fs::canonicalize(&attempt.path).expect("canonical attempt");
        assert_eq!(validate_attempt_directory(&canonical), Ok(()));
    }

    #[test]
    fn real_programmatic_gstreamer_analysis_profiles_emit_bounded_json() {
        let attempt = NativeAttempt::create().expect("attempt");
        let source = attempt.source_path(0);
        frame_media::record_synthetic_webm(&source).expect("fixture");
        assert_eq!(
            ensure_profile_runtime(NativeProfile::Probe, "application/json"),
            Ok(())
        );
        let authored = gst::parse::launch("filesrc ! decodebin")
            .expect("authored graph")
            .downcast::<gst::Pipeline>()
            .expect("pipeline");
        assert!(pipeline_has_declared_top_level_factories(&authored));
        for profile in [
            NativeProfile::Probe,
            NativeProfile::AudioPresence,
            NativeProfile::Waveform,
        ] {
            let plan = analysis_plan(
                profile,
                fs::metadata(&source).expect("metadata").len(),
                checksum(&source),
            );
            let plan_path = attempt.write_plan(&plan).expect("plan path");
            execute_child_plan(&plan_path, &CancellationToken::new()).expect("execute");
            let output =
                validate_output(&attempt, &plan, &CancellationToken::new()).expect("output");
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(output.path).expect("output bytes"))
                    .expect("output json");
            assert_eq!(value["profile"], profile.profile_id());
            match profile {
                NativeProfile::Probe => {
                    assert_eq!(value["container"], "webm");
                    assert_eq!(value["video_codec"], "vp8");
                    assert_eq!(value["audio_codec"], "opus");
                    assert_eq!(value["width"], 320);
                    assert_eq!(value["height"], 180);
                    assert_eq!(value["frame_rate_numerator"], 30);
                    assert_eq!(value["frame_rate_denominator"], 1);
                    assert_eq!(value["track_count"], 2);
                    let bytes = fs::read(attempt.output_path(&plan)).expect("manifest");
                    let manifest: ProbeManifestV1 =
                        serde_json::from_slice(&bytes).expect("probe manifest");
                    assert_eq!(serde_json::to_vec(&manifest).expect("canonical"), bytes);
                }
                NativeProfile::AudioPresence => {
                    assert_eq!(value["has_audio"], true);
                    assert_eq!(value["track_count"], 2);
                }
                NativeProfile::Waveform => assert!(
                    value["waveform_milli"]
                        .as_array()
                        .is_some_and(|v| !v.is_empty())
                ),
                _ => unreachable!(),
            }
            fs::remove_file(attempt.output_path(&plan)).expect("remove output");
            fs::remove_file(plan_path).expect("remove plan");
        }
    }

    #[test]
    fn cancellation_and_attempt_drop_leave_no_publishable_output() {
        let attempt = NativeAttempt::create().expect("attempt");
        let path = attempt.path.clone();
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());
        let plan = analysis_plan(NativeProfile::Probe, 10, "1".repeat(64));
        assert_eq!(
            run_native_plan(&attempt, &plan, &cancellation),
            Err(NativeError::Cancelled)
        );
        drop(attempt);
        assert!(!path.exists());
    }
}
