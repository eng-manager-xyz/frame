//! Owned GStreamer graph for recording normalized BGRA screen frames.
//!
//! Native capture callbacks must never wait for an encoder. Callers first use
//! the bounded queue in `screen_capture`, then drain that queue into this
//! graph. Appsrc and the downstream queue are independently bounded and never
//! leak. A push that would cross an appsrc bound fails closed and terminalizes
//! the recording; only [`ScreenRecording::finish`] verifies every encoded
//! frame and atomically commits an output.

mod av;
mod output;
mod pump;

use std::{
    collections::BTreeSet,
    fs::File,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use thiserror::Error;

use crate::{
    AppSrcBufferLifetime, CancellationToken, ColorSpace, FrameMemory, FrameTimestamp, MediaError,
    NativeExecutionError, NativeStudioExportArtifact, NativeStudioExportProfile, PixelFormat,
    RuntimeCapability, ScreenAppSrcPlan, ScreenFrame, ScreenFramePayload, VideoFrameSpec,
    pipeline_has_only_declared_authored_factories, pipeline_has_trusted_factory_provenance,
    prepare_runtime, render_studio_export,
};
use output::{
    ExpectedVideo, OutputReservation, preflight_verification, verify_playable_webm,
    verify_playable_webm_file,
};

pub use crate::ScreenFramePayload as ScreenRecordingPayload;
pub use av::{
    F32StereoAudioChunk, ScreenAudioRecording, ScreenAudioRecordingArtifact,
    ScreenAudioRecordingIngressStatus, SystemAudioRecordingSpec,
};
pub use pump::{
    ScreenPumpCancellationTeardown, ScreenPumpError, ScreenPumpOutcome, ScreenPumpReport,
    ScreenPumpRetirementFailure, ScreenPumpTeardownStatus, ScreenPumpTerminalFailure,
    ScreenRecordingPump,
};

pub const MAX_SCREEN_RECORDING_LONG_EDGE: u32 = 1_920;
pub const MAX_SCREEN_RECORDING_SHORT_EDGE: u32 = 1_080;

/// The largest tightly packed BGRA frame accepted by the first production
/// slice: 1080p in either landscape or portrait orientation.
pub const MAX_SCREEN_RECORDING_FRAME_BYTES: u64 = 1_920_u64 * 1_080 * 4;

/// Per-stage byte budget. Appsrc and the downstream queue each enforce it.
pub const SCREEN_RECORDING_QUEUE_BYTES: u64 = 64 * 1024 * 1024;

/// Frame-count ceiling used when byte size does not impose a tighter bound.
pub const SCREEN_RECORDING_QUEUE_FRAMES: u64 = 8;

/// First-slice wall-clock media ceiling, aligned with the light native media
/// sandbox. A new recording must be started after four hours.
pub const MAX_SCREEN_RECORDING_DURATION_NS: u64 = 14_400_000_000_000;

/// Maximum encoded bytes retained by one first-slice display recording.
pub const MAX_SCREEN_RECORDING_OUTPUT_BYTES: u64 = 2_000_000_000;

/// Free-space reserve below which recording terminates before consuming the
/// operating system's remaining working space.
pub const MIN_SCREEN_RECORDING_FREE_BYTES: u64 = 512_000_000;

const SCREEN_RECORDING_QUEUE_TIME_NS: u64 = 500_000_000;
const SCREEN_RECORDING_FINISH_TIMEOUT: Duration = Duration::from_secs(30);
const BUS_POLL: Duration = Duration::from_millis(25);
const RESOURCE_CHECK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Error)]
pub enum ScreenRecordingError {
    #[error("screen recording configuration is invalid or unsupported")]
    InvalidConfiguration,
    #[error("screen recording frame is invalid for the configured stream")]
    InvalidFrame,
    #[error("screen recording frame sequence or timestamp is not monotonic")]
    NonMonotonicFrame,
    #[error("screen recording lifecycle transition is invalid")]
    InvalidLifecycle,
    #[error("screen recording output already exists")]
    OutputExists,
    #[error("required screen recording GStreamer factory is unavailable")]
    MissingFactory,
    #[error("screen recording runtime rejected the current process environment")]
    Runtime(#[source] MediaError),
    #[error("screen recording GStreamer graph is untrusted")]
    UntrustedFactory,
    #[error("screen recording GStreamer graph failed")]
    Pipeline,
    #[error("screen recording ingress reached its bounded capacity")]
    Backpressure,
    #[error("screen recording reached its duration, output, or disk-space bound")]
    ResourceLimit,
    #[error("screen recording output did not contain every submitted frame")]
    FrameLoss,
    #[error("screen recording finalization timed out")]
    Timeout,
    #[error("screen recording was cancelled")]
    Cancelled,
    #[error("screen recording output is not a playable VP8 WebM")]
    InvalidOutput,
    #[error("screen recording output ownership changed unexpectedly")]
    OutputOwnership,
    #[error("screen recording filesystem operation failed")]
    Filesystem(#[source] std::io::Error),
    #[error("screen recording export failed")]
    Export(#[source] NativeExecutionError),
    #[error("screen recording completed but graph teardown could not be confirmed")]
    TeardownUnconfirmed(#[source] Box<ScreenRecordingError>),
    #[error("screen recording operation and subsequent graph teardown both failed")]
    OperationAndTeardown {
        operation: Box<ScreenRecordingError>,
        teardown: Box<ScreenRecordingError>,
    },
}

/// Validated, fixed stream format for one screen recording.
///
/// The first production slice intentionally accepts only tightly packed CPU
/// BGRA in sRGB. Platform adapters must convert before crossing this boundary;
/// silently relabeling CoreVideo, NV12, or wide-gamut frames would corrupt the
/// artifact while appearing successful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRecordingSpec {
    frame: VideoFrameSpec,
    frame_bytes: u64,
    ingress_max_frames: u64,
    ingress_max_time_ns: u64,
    frame_rate_numerator: i32,
    frame_rate_denominator: i32,
}

impl ScreenRecordingSpec {
    pub fn new(frame: VideoFrameSpec) -> Result<Self, ScreenRecordingError> {
        frame
            .validate()
            .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
        if frame.pixel_format != PixelFormat::Bgra8
            || frame.color_space != ColorSpace::Srgb
            || frame.memory != FrameMemory::Cpu
            || frame.width.max(frame.height) > MAX_SCREEN_RECORDING_LONG_EDGE
            || frame.width.min(frame.height) > MAX_SCREEN_RECORDING_SHORT_EDGE
            || frame.nominal_frame_duration_ns > SCREEN_RECORDING_QUEUE_TIME_NS
        {
            return Err(ScreenRecordingError::InvalidConfiguration);
        }
        let frame_bytes = u64::from(frame.width)
            .checked_mul(u64::from(frame.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|bytes| *bytes <= MAX_SCREEN_RECORDING_FRAME_BYTES)
            .ok_or(ScreenRecordingError::InvalidConfiguration)?;
        let divisor = greatest_common_divisor(1_000_000_000, frame.nominal_frame_duration_ns);
        let frame_rate_numerator = i32::try_from(1_000_000_000 / divisor)
            .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
        let frame_rate_denominator = i32::try_from(frame.nominal_frame_duration_ns / divisor)
            .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
        let ingress_max_frames = SCREEN_RECORDING_QUEUE_FRAMES
            .min(SCREEN_RECORDING_QUEUE_BYTES / frame_bytes)
            .max(1);
        let ingress_max_time_ns = frame
            .nominal_frame_duration_ns
            .checked_mul(ingress_max_frames)
            .map(|value| value.min(SCREEN_RECORDING_QUEUE_TIME_NS))
            .filter(|value| *value > 0)
            .ok_or(ScreenRecordingError::InvalidConfiguration)?;
        Ok(Self {
            frame,
            frame_bytes,
            ingress_max_frames,
            ingress_max_time_ns,
            frame_rate_numerator,
            frame_rate_denominator,
        })
    }

    /// Validates the complete provider-neutral appsrc negotiation before a
    /// recording graph is allowed to consume its frames.
    pub fn from_appsrc_plan(plan: ScreenAppSrcPlan) -> Result<Self, ScreenRecordingError> {
        if plan.factory != "appsrc"
            || plan.required_runtime_capability != RuntimeCapability::AppSourceBridge
            || !plan.is_live
            || !plan.time_format
            || plan.do_timestamp
            || plan.block
            || plan.buffer_lifetime != AppSrcBufferLifetime::OwnedUntilDownstreamRelease
        {
            return Err(ScreenRecordingError::InvalidConfiguration);
        }
        Self::new(plan.frame_spec)
    }

    #[must_use]
    pub const fn frame(self) -> VideoFrameSpec {
        self.frame
    }

    #[must_use]
    pub const fn frame_bytes(self) -> u64 {
        self.frame_bytes
    }

    #[must_use]
    pub const fn ingress_max_frames(self) -> u64 {
        self.ingress_max_frames
    }

    #[must_use]
    pub const fn ingress_max_bytes(self) -> u64 {
        SCREEN_RECORDING_QUEUE_BYTES
    }

    #[must_use]
    pub const fn ingress_max_time_ns(self) -> u64 {
        self.ingress_max_time_ns
    }
}

/// One owned, tightly packed BGRA frame with caller-supplied media timing.
pub struct BgraScreenFrame {
    sequence: u64,
    timestamp: FrameTimestamp,
    pixels: Vec<u8>,
}

impl BgraScreenFrame {
    pub fn new(
        sequence: u64,
        timestamp: FrameTimestamp,
        pixels: Vec<u8>,
    ) -> Result<Self, ScreenRecordingError> {
        if sequence == 0
            || timestamp.duration_ns == 0
            || timestamp
                .pts_ns
                .checked_add(timestamp.duration_ns)
                .is_none()
            || pixels.exact_retained_bytes().is_none()
        {
            return Err(ScreenRecordingError::InvalidFrame);
        }
        Ok(Self {
            sequence,
            timestamp,
            pixels,
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
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }
}

impl std::fmt::Debug for BgraScreenFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BgraScreenFrame")
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field(
                "pixels",
                &format_args!("<{} bytes redacted>", self.pixels.len()),
            )
            .finish()
    }
}

/// Snapshot after one non-blocking appsrc submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRecordingIngressStatus {
    pub submitted_frames: u64,
    pub queued_frames: u64,
    pub queued_bytes: u64,
    pub queued_time_ns: u64,
    pub at_capacity: bool,
}

/// Independently verified playable screen-track artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenRecordingArtifact {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub submitted_frames: u64,
    pub encoded_frames: u64,
    pub first_pts_ns: u64,
    pub end_pts_ns: u64,
    pub encoded_duration_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingState {
    Running,
    EosSent,
    Failed(TerminalFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalFailure {
    InvalidFrame,
    NonMonotonicFrame,
    Backpressure,
    ResourceLimit,
    Pipeline,
}

impl TerminalFailure {
    const fn error(self) -> ScreenRecordingError {
        match self {
            Self::InvalidFrame => ScreenRecordingError::InvalidFrame,
            Self::NonMonotonicFrame => ScreenRecordingError::NonMonotonicFrame,
            Self::Backpressure => ScreenRecordingError::Backpressure,
            Self::ResourceLimit => ScreenRecordingError::ResourceLimit,
            Self::Pipeline => ScreenRecordingError::Pipeline,
        }
    }
}

enum RecordingOutput {
    Managed(OutputReservation),
    #[cfg(unix)]
    Preopened {
        artifact_path: PathBuf,
        file: File,
    },
}

impl RecordingOutput {
    fn retained_file(&self) -> Result<&File, ScreenRecordingError> {
        match self {
            Self::Managed(output) => output.retained_file(),
            #[cfg(unix)]
            Self::Preopened { file, .. } => Ok(file),
        }
    }

    fn retained_file_mut(&mut self) -> Result<&mut File, ScreenRecordingError> {
        match self {
            Self::Managed(output) => output.retained_file_mut(),
            #[cfg(unix)]
            Self::Preopened { file, .. } => Ok(file),
        }
    }

    fn verify_ownership(&self) -> Result<(), ScreenRecordingError> {
        match self {
            Self::Managed(output) => output.verify_staging_identity(),
            #[cfg(unix)]
            Self::Preopened { file, .. } => verify_preopened_file(file),
        }
    }

    fn commit(&mut self) -> Result<PathBuf, ScreenRecordingError> {
        match self {
            Self::Managed(output) => output.commit(),
            #[cfg(unix)]
            Self::Preopened {
                artifact_path,
                file,
            } => {
                file.sync_all().map_err(ScreenRecordingError::Filesystem)?;
                Ok(artifact_path.clone())
            }
        }
    }

    #[cfg(not(unix))]
    fn staging_path(&self) -> Result<&Path, ScreenRecordingError> {
        match self {
            Self::Managed(output) => Ok(output.staging_path()),
        }
    }
}

/// Single owner of a live BGRA appsrc-to-VP8/WebM graph.
pub struct ScreenRecording {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    output: RecordingOutput,
    spec: ScreenRecordingSpec,
    state: RecordingState,
    submitted_frames: u64,
    first_pts_ns: Option<u64>,
    last_sequence: Option<u64>,
    last_end_pts_ns: Option<u64>,
    timestamp_segment_offset_ns: u64,
    last_resource_check: Instant,
}

impl std::fmt::Debug for ScreenRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenRecording")
            .field("spec", &self.spec)
            .field("state", &self.state)
            .field("submitted_frames", &self.submitted_frames)
            .field(
                "timestamp_segment_offset_ns",
                &self.timestamp_segment_offset_ns,
            )
            .field("output", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl ScreenRecording {
    /// Starts the graph after preflighting its encoder and verifier factories.
    pub fn start(
        output_path: impl Into<PathBuf>,
        spec: ScreenRecordingSpec,
    ) -> Result<Self, ScreenRecordingError> {
        preflight_screen_recording_runtime()?;
        let output_path = output_path.into();
        let output = RecordingOutput::Managed(OutputReservation::for_filesink(output_path)?);
        Self::build(output, spec)
    }

    /// Starts a Unix recording against an already-created private regular
    /// file. GStreamer writes and the independent verifier both use this
    /// descriptor; `artifact_path` is presentation metadata only and is never
    /// opened by this type. The caller retains cleanup and atomic-publication
    /// authority for the file after [`Self::finish`].
    #[cfg(unix)]
    pub fn start_preopened(
        artifact_path: impl Into<PathBuf>,
        file: File,
        spec: ScreenRecordingSpec,
    ) -> Result<Self, ScreenRecordingError> {
        preflight_screen_recording_runtime()?;
        verify_preopened_file(&file)?;
        if file
            .metadata()
            .map_err(ScreenRecordingError::Filesystem)?
            .len()
            != 0
        {
            return Err(ScreenRecordingError::OutputExists);
        }
        Self::build(
            RecordingOutput::Preopened {
                artifact_path: artifact_path.into(),
                file,
            },
            spec,
        )
    }

    fn build(
        output: RecordingOutput,
        spec: ScreenRecordingSpec,
    ) -> Result<Self, ScreenRecordingError> {
        enforce_output_bounds(output.retained_file()?, true)?;
        #[cfg(unix)]
        let description = format!(
            concat!(
                "appsrc name=screen_recording_src ",
                "! queue name=screen_recording_queue max-size-buffers={} max-size-bytes={} ",
                "max-size-time={} leaky=no ",
                "! videoconvert ! vp8enc deadline=1 ",
                "! webmmux streamable=false ! fdsink name=screen_recording_sink sync=false"
            ),
            spec.ingress_max_frames, SCREEN_RECORDING_QUEUE_BYTES, spec.ingress_max_time_ns,
        );
        #[cfg(not(unix))]
        let description = format!(
            concat!(
                "appsrc name=screen_recording_src ",
                "! queue name=screen_recording_queue max-size-buffers={} max-size-bytes={} ",
                "max-size-time={} leaky=no ",
                "! videoconvert ! vp8enc deadline=1 ",
                "! webmmux streamable=false ! filesink name=screen_recording_sink sync=false"
            ),
            spec.ingress_max_frames, SCREEN_RECORDING_QUEUE_BYTES, spec.ingress_max_time_ns,
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|_| ScreenRecordingError::Pipeline)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| ScreenRecordingError::Pipeline)?;
        require_trusted(&pipeline)?;
        let appsrc = pipeline
            .by_name("screen_recording_src")
            .ok_or(ScreenRecordingError::Pipeline)?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| ScreenRecordingError::Pipeline)?;
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("colorimetry", "sRGB")
            .field(
                "width",
                i32::try_from(spec.frame.width)
                    .map_err(|_| ScreenRecordingError::InvalidConfiguration)?,
            )
            .field(
                "height",
                i32::try_from(spec.frame.height)
                    .map_err(|_| ScreenRecordingError::InvalidConfiguration)?,
            )
            .field(
                "framerate",
                gst::Fraction::new(spec.frame_rate_numerator, spec.frame_rate_denominator),
            )
            .build();
        appsrc.set_caps(Some(&caps));
        appsrc.set_is_live(true);
        appsrc.set_format(gst::Format::Time);
        appsrc.set_do_timestamp(false);
        appsrc.set_block(false);
        appsrc.set_max_buffers(spec.ingress_max_frames);
        appsrc.set_max_bytes(SCREEN_RECORDING_QUEUE_BYTES);
        appsrc.set_max_time(gst::ClockTime::from_nseconds(spec.ingress_max_time_ns));
        appsrc.set_leaky_type(gst_app::AppLeakyType::None);
        let sink = pipeline
            .by_name("screen_recording_sink")
            .ok_or(ScreenRecordingError::Pipeline)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            sink.set_property("fd", output.retained_file()?.as_raw_fd());
        }
        #[cfg(not(unix))]
        sink.set_property("location", output.staging_path()?);
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| ScreenRecordingError::Pipeline)?;
        Ok(Self {
            pipeline,
            appsrc,
            output,
            spec,
            state: RecordingState::Running,
            submitted_frames: 0,
            first_pts_ns: None,
            last_sequence: None,
            last_end_pts_ns: None,
            timestamp_segment_offset_ns: 0,
            last_resource_check: Instant::now(),
        })
    }

    #[must_use]
    pub const fn spec(&self) -> ScreenRecordingSpec {
        self.spec
    }

    /// Returns the current bounded appsrc occupancy without waiting for the
    /// encoder. A single owner can use this snapshot to defer its next drain
    /// attempt instead of submitting a frame that would terminalize on
    /// backpressure.
    #[must_use]
    pub fn ingress_status(&self) -> ScreenRecordingIngressStatus {
        let ingress = self.ingress_levels();
        ScreenRecordingIngressStatus {
            submitted_frames: self.submitted_frames,
            queued_frames: ingress.frames,
            queued_bytes: ingress.bytes,
            queued_time_ns: ingress.time_ns,
            at_capacity: ingress.frames >= self.spec.ingress_max_frames
                || ingress.bytes >= SCREEN_RECORDING_QUEUE_BYTES
                || ingress.time_ns >= self.spec.ingress_max_time_ns,
        }
    }

    /// Number of frames accepted by this graph since it started.
    #[must_use]
    pub const fn submitted_frames(&self) -> u64 {
        self.submitted_frames
    }

    /// True only before this live graph has accepted input, observed a
    /// terminal bus failure, or begun EOS. Pump construction uses the full
    /// predicate so a zero-frame but already-terminal graph cannot be claimed
    /// as a fresh recording segment.
    pub(crate) fn is_pristine_running(&mut self) -> bool {
        if matches!(self.state, RecordingState::Running) && self.pipeline_failed() {
            self.state = RecordingState::Failed(TerminalFailure::Pipeline);
        }
        let ingress = self.ingress_levels();
        matches!(self.state, RecordingState::Running)
            && self.submitted_frames == 0
            && self.first_pts_ns.is_none()
            && self.last_sequence.is_none()
            && self.last_end_pts_ns.is_none()
            && self.timestamp_segment_offset_ns == 0
            && ingress.frames == 0
            && ingress.bytes == 0
            && ingress.time_ns == 0
    }

    /// Submits one frame without waiting for downstream encoding.
    ///
    /// No queue leaks. If this frame would cross any appsrc ceiling, the call
    /// returns [`ScreenRecordingError::Backpressure`] and the recording becomes
    /// terminal so a caller cannot accidentally commit a partial timeline.
    pub fn push_frame(
        &mut self,
        frame: BgraScreenFrame,
    ) -> Result<ScreenRecordingIngressStatus, ScreenRecordingError> {
        self.push_owned_payload(frame.sequence, frame.timestamp, frame.pixels)
    }

    fn push_owned_payload<P>(
        &mut self,
        sequence: u64,
        mut timestamp: FrameTimestamp,
        pixels: P,
    ) -> Result<ScreenRecordingIngressStatus, ScreenRecordingError>
    where
        P: ScreenRecordingPayload,
    {
        match self.state {
            RecordingState::Running => {}
            RecordingState::EosSent => return Err(ScreenRecordingError::InvalidLifecycle),
            RecordingState::Failed(failure) => return Err(failure.error()),
        }
        if self.pipeline_failed() {
            return self.fail(TerminalFailure::Pipeline);
        }
        let expected_bytes = usize::try_from(self.spec.frame_bytes)
            .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
        if pixels.exact_retained_bytes() != Some(self.spec.frame_bytes)
            || pixels.as_ref().len() != expected_bytes
        {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        if self
            .last_sequence
            .is_some_and(|previous| sequence <= previous)
        {
            return self.fail(TerminalFailure::NonMonotonicFrame);
        }
        let timestamp_segment_offset_ns = if timestamp.discontinuity {
            self.last_end_pts_ns
                .and_then(|previous_end_pts_ns| previous_end_pts_ns.checked_sub(timestamp.pts_ns))
                .unwrap_or(0)
        } else {
            self.timestamp_segment_offset_ns
        };
        let Some(adjusted_pts_ns) = timestamp.pts_ns.checked_add(timestamp_segment_offset_ns)
        else {
            return self.fail(TerminalFailure::InvalidFrame);
        };
        timestamp.pts_ns = adjusted_pts_ns;
        if self
            .last_end_pts_ns
            .is_some_and(|previous_end_pts_ns| timestamp.pts_ns < previous_end_pts_ns)
        {
            return self.fail(TerminalFailure::NonMonotonicFrame);
        }
        if timestamp
            .pts_ns
            .checked_add(timestamp.duration_ns)
            .is_none()
        {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        let first_pts_ns = self.first_pts_ns.unwrap_or(timestamp.pts_ns);
        if timestamp
            .end_ns()
            .checked_sub(first_pts_ns)
            .is_none_or(|duration| duration > MAX_SCREEN_RECORDING_DURATION_NS)
        {
            return self.fail(TerminalFailure::ResourceLimit);
        }
        let check_free_space = self.last_resource_check.elapsed() >= RESOURCE_CHECK_INTERVAL;
        if self
            .output
            .retained_file()
            .and_then(|file| enforce_output_bounds(file, check_free_space))
            .is_err()
        {
            return self.fail(TerminalFailure::ResourceLimit);
        }
        if check_free_space {
            self.last_resource_check = Instant::now();
        }
        let ingress = self.ingress_levels();
        if would_exceed_ingress(
            self.spec,
            ingress,
            self.spec.frame_bytes,
            timestamp.duration_ns,
        ) {
            return self.fail(TerminalFailure::Backpressure);
        }
        let buffer = match build_owned_buffer(sequence, timestamp, pixels) {
            Ok(buffer) => buffer,
            Err(_) => return self.fail(TerminalFailure::InvalidFrame),
        };
        if self.appsrc.push_buffer(buffer).is_err() {
            return self.fail(TerminalFailure::Pipeline);
        }
        let Some(submitted_frames) = self.submitted_frames.checked_add(1) else {
            return self.fail(TerminalFailure::InvalidFrame);
        };
        self.submitted_frames = submitted_frames;
        self.first_pts_ns.get_or_insert(timestamp.pts_ns);
        self.last_sequence = Some(sequence);
        self.last_end_pts_ns = Some(timestamp.end_ns());
        self.timestamp_segment_offset_ns = timestamp_segment_offset_ns;
        Ok(self.ingress_status())
    }

    /// Pushes a normalized screen-capture frame without copying its BGRA body.
    pub fn push_screen_frame<P>(
        &mut self,
        frame: ScreenFrame<P>,
    ) -> Result<ScreenRecordingIngressStatus, ScreenRecordingError>
    where
        P: ScreenRecordingPayload,
    {
        if frame.spec() != self.spec.frame || frame.retained_bytes() != self.spec.frame_bytes {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        let sequence = frame.sequence();
        let timestamp = frame.timestamp();
        self.push_owned_payload(sequence, timestamp, frame.into_payload())
    }

    /// Signals that no more frames will arrive. Repeating EOS is idempotent.
    pub fn end_of_stream(&mut self) -> Result<(), ScreenRecordingError> {
        match self.state {
            RecordingState::Running => {
                if self.pipeline_failed() {
                    return self.fail(TerminalFailure::Pipeline);
                }
                if self.appsrc.end_of_stream().is_err() {
                    return self.fail(TerminalFailure::Pipeline);
                }
                self.state = RecordingState::EosSent;
                Ok(())
            }
            RecordingState::EosSent => Ok(()),
            RecordingState::Failed(failure) => Err(failure.error()),
        }
    }

    /// Stops the graph without publishing an artifact and confirms the Null
    /// state before returning. Cancellation and failure paths use this instead
    /// of relying on `Drop`, whose teardown result cannot be observed.
    pub fn abort(self) -> Result<(), ScreenRecordingError> {
        set_null(&self.pipeline)
    }

    /// Waits for muxer EOS, reaches Null, and independently decodes the WebM.
    pub fn finish(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<ScreenRecordingArtifact, ScreenRecordingError> {
        let lifecycle_error = match self.state {
            RecordingState::EosSent if self.submitted_frames > 0 => None,
            RecordingState::Failed(failure) => Some(failure.error()),
            RecordingState::Running | RecordingState::EosSent => {
                Some(ScreenRecordingError::InvalidLifecycle)
            }
        };
        if let Some(operation) = lifecycle_error {
            return match set_null(&self.pipeline) {
                Ok(()) => Err(operation),
                Err(teardown) => Err(ScreenRecordingError::OperationAndTeardown {
                    operation: Box::new(operation),
                    teardown: Box::new(teardown),
                }),
            };
        }
        let terminal = wait_for_eos(
            &self.pipeline,
            cancellation,
            SCREEN_RECORDING_FINISH_TIMEOUT,
        );
        let teardown = set_null(&self.pipeline);
        classify_terminal_and_teardown(terminal, teardown)?;
        self.output.verify_ownership()?;
        enforce_output_bounds(self.output.retained_file()?, true)?;
        let first_input_pts_ns = self
            .first_pts_ns
            .ok_or(ScreenRecordingError::InvalidLifecycle)?;
        let input_end_pts_ns = self
            .last_end_pts_ns
            .ok_or(ScreenRecordingError::InvalidLifecycle)?;
        let expected_duration_ns = input_end_pts_ns
            .checked_sub(first_input_pts_ns)
            .ok_or(ScreenRecordingError::InvalidLifecycle)?;
        let expected = Some(ExpectedVideo {
            frames: self.submitted_frames,
            duration_ns: expected_duration_ns,
        });
        #[cfg(unix)]
        let verified = verify_playable_webm_file(
            self.output.retained_file_mut()?,
            cancellation,
            expected,
            true,
        )?;
        #[cfg(not(unix))]
        let verified =
            verify_playable_webm(self.output.staging_path()?, cancellation, expected, true)?;
        self.output.verify_ownership()?;
        enforce_output_bounds(self.output.retained_file()?, true)?;
        let path = self.output.commit()?;
        Ok(ScreenRecordingArtifact {
            path,
            bytes: verified.bytes,
            sha256: verified.sha256.ok_or(ScreenRecordingError::InvalidOutput)?,
            submitted_frames: self.submitted_frames,
            encoded_frames: verified.encoded_frames,
            first_pts_ns: verified.first_pts_ns,
            end_pts_ns: verified.end_pts_ns,
            encoded_duration_ns: verified.encoded_duration_ns,
        })
    }

    fn ingress_levels(&self) -> IngressLevels {
        IngressLevels {
            frames: self.appsrc.current_level_buffers(),
            bytes: self.appsrc.current_level_bytes(),
            time_ns: self.appsrc.current_level_time().nseconds(),
        }
    }

    fn pipeline_failed(&self) -> bool {
        self.pipeline
            .bus()
            .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
            .is_some()
    }

    fn fail<T>(&mut self, failure: TerminalFailure) -> Result<T, ScreenRecordingError> {
        self.state = RecordingState::Failed(failure);
        Err(failure.error())
    }
}

fn classify_terminal_and_teardown(
    terminal: Result<(), ScreenRecordingError>,
    teardown: Result<(), ScreenRecordingError>,
) -> Result<(), ScreenRecordingError> {
    match (terminal, teardown) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(operation), Ok(())) => Err(operation),
        (Ok(()), Err(teardown)) => Err(ScreenRecordingError::TeardownUnconfirmed(Box::new(
            teardown,
        ))),
        (Err(operation), Err(teardown)) => Err(ScreenRecordingError::OperationAndTeardown {
            operation: Box::new(operation),
            teardown: Box::new(teardown),
        }),
    }
}

/// Proves that the current process can build and independently verify the
/// production VP8/WebM screen graph without creating an output artifact.
///
/// Composition roots use this before advertising a native recorder. A later
/// recording start repeats the check so runtime or environment drift still
/// fails closed.
pub fn preflight_screen_recording_runtime() -> Result<(), ScreenRecordingError> {
    prepare_runtime().map_err(ScreenRecordingError::Runtime)?;
    require_factories()
}

impl Drop for ScreenRecording {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IngressLevels {
    frames: u64,
    bytes: u64,
    time_ns: u64,
}

fn would_exceed_ingress(
    spec: ScreenRecordingSpec,
    levels: IngressLevels,
    frame_bytes: u64,
    frame_duration_ns: u64,
) -> bool {
    levels
        .frames
        .checked_add(1)
        .is_none_or(|value| value > spec.ingress_max_frames)
        || levels
            .bytes
            .checked_add(frame_bytes)
            .is_none_or(|value| value > SCREEN_RECORDING_QUEUE_BYTES)
        || levels
            .time_ns
            .checked_add(frame_duration_ns)
            .is_none_or(|value| value > spec.ingress_max_time_ns)
}

/// Re-encodes a screen track through the existing native WebM export path and
/// then independently decodes the resulting artifact before returning it.
pub fn export_screen_recording_webm(
    source: &Path,
    output: &Path,
    cancellation: &CancellationToken,
) -> Result<NativeStudioExportArtifact, ScreenRecordingError> {
    prepare_runtime().map_err(ScreenRecordingError::Runtime)?;
    preflight_verification()?;
    let mut reservation = OutputReservation::for_external_writer(output.to_path_buf())?;
    let mut artifact = match render_studio_export(
        source,
        reservation.staging_path(),
        NativeStudioExportProfile::EditableWebM,
        cancellation,
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            let _ = reservation.adopt_created();
            return Err(ScreenRecordingError::Export(error));
        }
    };
    reservation.adopt_created()?;
    let verified = verify_playable_webm(reservation.staging_path(), cancellation, None, false)?;
    reservation.verify_staging_identity()?;
    if verified.bytes != artifact.bytes {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    artifact.path = reservation.commit()?;
    Ok(artifact)
}

#[cfg(test)]
fn build_buffer(frame: BgraScreenFrame) -> Result<gst::Buffer, ScreenRecordingError> {
    build_owned_buffer(frame.sequence, frame.timestamp, frame.pixels)
}

fn build_owned_buffer<P>(
    sequence: u64,
    timestamp: FrameTimestamp,
    pixels: P,
) -> Result<gst::Buffer, ScreenRecordingError>
where
    P: ScreenRecordingPayload,
{
    pixels
        .exact_retained_bytes()
        .ok_or(ScreenRecordingError::InvalidFrame)?;
    let mut buffer = gst::Buffer::from_slice(pixels);
    let buffer_ref = buffer.get_mut().ok_or(ScreenRecordingError::InvalidFrame)?;
    buffer_ref.set_pts(gst::ClockTime::from_nseconds(timestamp.pts_ns));
    buffer_ref.set_duration(gst::ClockTime::from_nseconds(timestamp.duration_ns));
    buffer_ref.set_offset(sequence);
    buffer_ref.set_offset_end(
        sequence
            .checked_add(1)
            .ok_or(ScreenRecordingError::InvalidFrame)?,
    );
    if timestamp.discontinuity {
        buffer_ref.set_flags(gst::BufferFlags::DISCONT);
    }
    Ok(buffer)
}

fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(unix)]
fn verify_preopened_file(file: &File) -> Result<(), ScreenRecordingError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = file.metadata().map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    Ok(())
}

fn enforce_output_bounds(file: &File, check_free_space: bool) -> Result<(), ScreenRecordingError> {
    let metadata = file.metadata().map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SCREEN_RECORDING_OUTPUT_BYTES {
        return Err(ScreenRecordingError::ResourceLimit);
    }
    #[cfg(unix)]
    if check_free_space {
        let filesystem = rustix::fs::fstatvfs(file)
            .map_err(|error| ScreenRecordingError::Filesystem(error.into()))?;
        let fragment_size = if filesystem.f_frsize == 0 {
            filesystem.f_bsize
        } else {
            filesystem.f_frsize
        };
        let available = filesystem
            .f_bavail
            .checked_mul(fragment_size)
            .ok_or(ScreenRecordingError::ResourceLimit)?;
        if available < MIN_SCREEN_RECORDING_FREE_BYTES {
            return Err(ScreenRecordingError::ResourceLimit);
        }
    }
    #[cfg(not(unix))]
    let _ = check_free_space;
    Ok(())
}

fn require_trusted(pipeline: &gst::Pipeline) -> Result<(), ScreenRecordingError> {
    if !pipeline_has_trusted_factory_provenance(pipeline)
        || !pipeline_has_only_declared_authored_factories(pipeline)
    {
        return Err(ScreenRecordingError::UntrustedFactory);
    }
    let factories = pipeline
        .children()
        .iter()
        .filter_map(|element| element.factory().map(|factory| factory.name().to_string()))
        .collect::<BTreeSet<_>>();
    if factories.is_empty() {
        return Err(ScreenRecordingError::Pipeline);
    }
    Ok(())
}

fn require_factories() -> Result<(), ScreenRecordingError> {
    #[cfg(unix)]
    let sink = "fdsink";
    #[cfg(not(unix))]
    let sink = "filesink";
    for name in ["appsrc", "queue", "videoconvert", "vp8enc", "webmmux", sink] {
        if gst::ElementFactory::find(name).is_none() {
            return Err(ScreenRecordingError::MissingFactory);
        }
    }
    preflight_verification()
}

fn wait_for_eos(
    pipeline: &gst::Pipeline,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<(), ScreenRecordingError> {
    let bus = pipeline.bus().ok_or(ScreenRecordingError::Pipeline)?;
    let started = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            return Err(ScreenRecordingError::Cancelled);
        }
        if started.elapsed() >= timeout {
            return Err(ScreenRecordingError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL.as_millis() as u64),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => return Ok(()),
            gst::MessageView::Error(_) => return Err(ScreenRecordingError::Pipeline),
            _ => {}
        }
    }
}

fn set_null(pipeline: &gst::Pipeline) -> Result<(), ScreenRecordingError> {
    pipeline
        .set_state(gst::State::Null)
        .map_err(|_| ScreenRecordingError::Pipeline)?;
    let (state_change, current, pending) = pipeline.state(gst::ClockTime::from_seconds(5));
    if state_change.is_err() || current != gst::State::Null || pending != gst::State::VoidPending {
        return Err(ScreenRecordingError::Pipeline);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct CountedPixels {
        bytes: Vec<u8>,
        drops: Arc<AtomicUsize>,
    }

    impl AsRef<[u8]> for CountedPixels {
        fn as_ref(&self) -> &[u8] {
            &self.bytes
        }
    }

    // The sidecar exists only in this private test implementation so the test
    // can observe GStreamer's final release. Shipping callers remain limited
    // to the two exact-accounting implementations above.
    impl crate::screen_capture::screen_payload_seal::Sealed for CountedPixels {}

    impl ScreenFramePayload for CountedPixels {
        fn exact_retained_bytes(&self) -> Option<u64> {
            u64::try_from(self.bytes.len()).ok()
        }
    }

    impl Drop for CountedPixels {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn test_frame_spec() -> VideoFrameSpec {
        VideoFrameSpec {
            width: 320,
            height: 180,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: FrameMemory::Cpu,
        }
    }

    fn test_appsrc_plan() -> ScreenAppSrcPlan {
        ScreenAppSrcPlan {
            factory: "appsrc",
            required_runtime_capability: RuntimeCapability::AppSourceBridge,
            is_live: true,
            time_format: true,
            do_timestamp: false,
            block: false,
            buffer_lifetime: AppSrcBufferLifetime::OwnedUntilDownstreamRelease,
            frame_spec: test_frame_spec(),
            queue: crate::ScreenCaptureQueuePolicy::new(
                8,
                SCREEN_RECORDING_QUEUE_BYTES,
                500_000_000,
                crate::CaptureQueueOverflow::DropOldest,
            )
            .expect("bounded test queue"),
        }
    }

    #[test]
    fn buffer_preserves_explicit_timing_and_discontinuity() {
        gst::init().expect("GStreamer runtime");
        let timestamp = FrameTimestamp {
            pts_ns: 50,
            duration_ns: 10,
            discontinuity: true,
        };
        let buffer = build_buffer(
            BgraScreenFrame::new(7, timestamp, vec![0; 16]).expect("valid BGRA frame"),
        )
        .expect("GStreamer buffer");
        assert_eq!(buffer.pts().map(gst::ClockTime::nseconds), Some(50));
        assert_eq!(buffer.duration().map(gst::ClockTime::nseconds), Some(10));
        assert_eq!(buffer.offset(), 7);
        assert_eq!(buffer.offset_end(), 8);
        assert!(buffer.flags().contains(gst::BufferFlags::DISCONT));
    }

    #[test]
    fn buffer_retains_owned_payload_until_the_last_gstreamer_reference_drops() {
        gst::init().expect("GStreamer runtime");
        let drops = Arc::new(AtomicUsize::new(0));
        let buffer = build_owned_buffer(
            1,
            FrameTimestamp::new(0, 10).expect("timestamp"),
            CountedPixels {
                bytes: vec![0; 16],
                drops: Arc::clone(&drops),
            },
        )
        .expect("owned GStreamer buffer");
        let retained = buffer.clone();
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(buffer);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(retained);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn payload_contract_accepts_exact_box_and_rejects_vec_spare_capacity() {
        gst::init().expect("GStreamer runtime");
        let buffer = build_owned_buffer(
            1,
            FrameTimestamp::new(0, 10).expect("timestamp"),
            vec![0; 16].into_boxed_slice(),
        )
        .expect("boxed slices retain exactly their visible bytes");
        assert_eq!(buffer.size(), 16);

        let mut pixels = Vec::with_capacity(17);
        pixels.resize(16, 0);
        assert!(pixels.capacity() > pixels.len());
        assert!(matches!(
            build_owned_buffer(2, FrameTimestamp::new(10, 10).expect("timestamp"), pixels,),
            Err(ScreenRecordingError::InvalidFrame)
        ));

        let mut pixels = Vec::with_capacity(17);
        pixels.resize(16, 0);
        assert!(matches!(
            BgraScreenFrame::new(1, FrameTimestamp::new(0, 10).expect("timestamp"), pixels,),
            Err(ScreenRecordingError::InvalidFrame)
        ));
    }

    #[test]
    fn appsrc_plan_validation_is_exact_and_caps_declare_srgb() {
        let plan = test_appsrc_plan();
        let spec = ScreenRecordingSpec::from_appsrc_plan(plan).expect("exact appsrc plan");
        assert_eq!(spec.frame(), test_frame_spec());

        let mut invalid = plan;
        invalid.do_timestamp = true;
        assert!(matches!(
            ScreenRecordingSpec::from_appsrc_plan(invalid),
            Err(ScreenRecordingError::InvalidConfiguration)
        ));

        let directory = tempfile::tempdir().expect("temporary recording directory");
        let recording = ScreenRecording::start(directory.path().join("caps.webm"), spec)
            .expect("recording graph");
        let caps = recording.appsrc.caps().expect("appsrc caps");
        let structure = caps.structure(0).expect("raw-video caps structure");
        assert_eq!(structure.get::<&str>("format"), Ok("BGRA"));
        assert_eq!(structure.get::<&str>("colorimetry"), Ok("sRGB"));
        recording.abort().expect("confirmed Null teardown");
    }

    #[test]
    fn spec_accepts_1080p_and_rejects_oversized_frames() {
        let spec = ScreenRecordingSpec::new(VideoFrameSpec {
            width: 1_920,
            height: 1_080,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: FrameMemory::Cpu,
        })
        .expect("1080p remains inside the first production slice");
        assert_eq!(spec.frame_bytes(), 8_294_400);
        assert_eq!(spec.ingress_max_frames(), 8);
        assert!(spec.ingress_max_bytes() >= spec.frame_bytes());

        assert!(matches!(
            ScreenRecordingSpec::new(VideoFrameSpec {
                width: 3_840,
                height: 2_160,
                pixel_format: PixelFormat::Bgra8,
                color_space: ColorSpace::Srgb,
                nominal_frame_duration_ns: 33_333_333,
                memory: FrameMemory::Cpu,
            }),
            Err(ScreenRecordingError::InvalidConfiguration)
        ));
    }

    #[test]
    fn ingress_rejects_before_crossing_each_bound() {
        let spec = ScreenRecordingSpec::new(VideoFrameSpec {
            width: 320,
            height: 180,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: FrameMemory::Cpu,
        })
        .expect("bounded recording spec");
        assert!(!would_exceed_ingress(
            spec,
            IngressLevels {
                frames: spec.ingress_max_frames() - 1,
                bytes: 0,
                time_ns: 0,
            },
            spec.frame_bytes(),
            spec.frame.nominal_frame_duration_ns,
        ));
        assert!(would_exceed_ingress(
            spec,
            IngressLevels {
                frames: spec.ingress_max_frames(),
                bytes: 0,
                time_ns: 0,
            },
            spec.frame_bytes(),
            spec.frame.nominal_frame_duration_ns,
        ));
        assert!(would_exceed_ingress(
            spec,
            IngressLevels {
                frames: 0,
                bytes: SCREEN_RECORDING_QUEUE_BYTES - spec.frame_bytes() + 1,
                time_ns: 0,
            },
            spec.frame_bytes(),
            spec.frame.nominal_frame_duration_ns,
        ));
        assert!(would_exceed_ingress(
            spec,
            IngressLevels {
                frames: 0,
                bytes: 0,
                time_ns: spec.ingress_max_time_ns(),
            },
            spec.frame_bytes(),
            spec.frame.nominal_frame_duration_ns,
        ));
    }

    #[test]
    fn recording_preflight_includes_every_verifier_factory() {
        #[cfg(unix)]
        let source = "fdsrc";
        #[cfg(not(unix))]
        let source = "filesrc";
        for required in [source, "matroskademux", "identity", "vp8dec", "fakesink"] {
            assert!(output::VERIFICATION_FACTORIES.contains(&required));
        }
    }

    #[test]
    fn recording_duration_bound_is_exact() {
        let first_pts_ns = 50_u64;
        assert_eq!(
            first_pts_ns
                .checked_add(MAX_SCREEN_RECORDING_DURATION_NS)
                .and_then(|end| end.checked_sub(first_pts_ns)),
            Some(MAX_SCREEN_RECORDING_DURATION_NS)
        );
        assert!(
            first_pts_ns
                .checked_add(MAX_SCREEN_RECORDING_DURATION_NS + 1)
                .and_then(|end| end.checked_sub(first_pts_ns))
                .is_some_and(|duration| duration > MAX_SCREEN_RECORDING_DURATION_NS)
        );
    }

    #[test]
    fn terminal_success_does_not_hide_unconfirmed_teardown() {
        let error = classify_terminal_and_teardown(Ok(()), Err(ScreenRecordingError::Pipeline))
            .expect_err("teardown failure remains observable");
        assert!(matches!(
            error,
            ScreenRecordingError::TeardownUnconfirmed(teardown)
                if matches!(*teardown, ScreenRecordingError::Pipeline)
        ));
    }
}
