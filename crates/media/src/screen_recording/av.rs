//! Bounded screen plus system-audio recording on one GStreamer clock.
//!
//! The two native producers retain their own callback queues, while this owner
//! enforces independent non-blocking appsrc budgets and one mux/EOS authority.

use std::{path::PathBuf, time::Instant};

#[cfg(unix)]
use std::fs::File;

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;

#[cfg(not(unix))]
use super::output::verify_playable_av_webm;
use super::output::{
    ExpectedAudio, ExpectedVideo, OutputReservation, preflight_av_verification,
    verify_playable_av_webm_file,
};
#[cfg(unix)]
use super::verify_preopened_file;
use super::{
    BgraScreenFrame, MAX_SCREEN_RECORDING_DURATION_NS, RESOURCE_CHECK_INTERVAL, RecordingOutput,
    SCREEN_RECORDING_FINISH_TIMEOUT, SCREEN_RECORDING_QUEUE_BYTES, ScreenRecordingError,
    ScreenRecordingSpec, classify_terminal_and_teardown, enforce_output_bounds, require_trusted,
    set_null, wait_for_eos,
};
use crate::{AudioFormat, AudioSampleFormat, CancellationToken, FrameTimestamp, prepare_runtime};

const AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
const AUDIO_CHANNELS: u8 = 2;
const AUDIO_BYTES_PER_FRAME: u64 = 8;
const MAX_AUDIO_CHUNK_FRAMES: u64 = 4_800;
const MAX_AUDIO_CHUNK_BYTES: u64 = MAX_AUDIO_CHUNK_FRAMES * AUDIO_BYTES_PER_FRAME;
const AUDIO_QUEUE_BUFFERS: u64 = 128;
const AUDIO_QUEUE_BYTES: u64 = 8 * 1024 * 1024;
const AUDIO_QUEUE_TIME_NS: u64 = 2_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemAudioRecordingSpec {
    format: AudioFormat,
}

impl SystemAudioRecordingSpec {
    pub fn new(format: AudioFormat) -> Result<Self, ScreenRecordingError> {
        if format.sample_rate != AUDIO_SAMPLE_RATE_HZ
            || format.channels != AUDIO_CHANNELS
            || format.sample_format != AudioSampleFormat::Float32
        {
            return Err(ScreenRecordingError::InvalidConfiguration);
        }
        Ok(Self { format })
    }

    #[must_use]
    pub const fn format(self) -> AudioFormat {
        self.format
    }
}

pub struct F32StereoAudioChunk {
    sequence: u64,
    timestamp: FrameTimestamp,
    samples: Vec<u8>,
}

impl F32StereoAudioChunk {
    pub fn new(
        sequence: u64,
        source_pts_ns: u64,
        duration_ns: u64,
        discontinuity: bool,
        samples: Vec<u8>,
    ) -> Result<Self, ScreenRecordingError> {
        let bytes = u64::try_from(samples.len()).map_err(|_| ScreenRecordingError::InvalidFrame)?;
        let frames = bytes
            .checked_div(AUDIO_BYTES_PER_FRAME)
            .filter(|frames| {
                *frames > 0
                    && *frames <= MAX_AUDIO_CHUNK_FRAMES
                    && frames.checked_mul(AUDIO_BYTES_PER_FRAME) == Some(bytes)
            })
            .ok_or(ScreenRecordingError::InvalidFrame)?;
        let expected_duration = frames
            .checked_mul(1_000_000_000)
            .and_then(|value| value.checked_div(u64::from(AUDIO_SAMPLE_RATE_HZ)))
            .filter(|value| *value > 0)
            .ok_or(ScreenRecordingError::InvalidFrame)?;
        if sequence == 0
            || duration_ns != expected_duration
            || source_pts_ns.checked_add(duration_ns).is_none()
            || samples
                .as_chunks::<4>()
                .0
                .iter()
                .any(|sample| !f32::from_le_bytes(*sample).is_finite())
        {
            return Err(ScreenRecordingError::InvalidFrame);
        }
        Ok(Self {
            sequence,
            timestamp: FrameTimestamp {
                pts_ns: source_pts_ns,
                duration_ns,
                discontinuity,
            },
            samples,
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
        self.samples.len()
    }
}

impl std::fmt::Debug for F32StereoAudioChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("F32StereoAudioChunk")
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field("retained_bytes", &self.samples.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenAudioRecordingIngressStatus {
    pub submitted_video_frames: u64,
    pub submitted_audio_chunks: u64,
    pub queued_buffers: u64,
    pub queued_bytes: u64,
    pub queued_time_ns: u64,
    pub at_capacity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenAudioRecordingArtifact {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub submitted_video_frames: u64,
    pub encoded_video_frames: u64,
    pub submitted_audio_chunks: u64,
    pub decoded_audio_buffers: u64,
    pub video_duration_ns: u64,
    pub audio_duration_ns: u64,
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

#[derive(Debug, Clone, Copy)]
struct SourceProgress {
    submitted: u64,
    first_pts_ns: Option<u64>,
    last_sequence: Option<u64>,
    last_end_pts_ns: Option<u64>,
}

impl SourceProgress {
    const fn new() -> Self {
        Self {
            submitted: 0,
            first_pts_ns: None,
            last_sequence: None,
            last_end_pts_ns: None,
        }
    }

    fn validate_next(
        &self,
        sequence: u64,
        timestamp: FrameTimestamp,
    ) -> Result<(), TerminalFailure> {
        if sequence == 0
            || self
                .last_sequence
                .is_some_and(|previous| sequence <= previous)
            || self
                .last_end_pts_ns
                .is_some_and(|previous| timestamp.pts_ns < previous)
            || timestamp.duration_ns == 0
            || timestamp
                .pts_ns
                .checked_add(timestamp.duration_ns)
                .is_none()
        {
            return Err(TerminalFailure::NonMonotonicFrame);
        }
        let first = self.first_pts_ns.unwrap_or(timestamp.pts_ns);
        if timestamp
            .end_ns()
            .checked_sub(first)
            .is_none_or(|duration| duration > MAX_SCREEN_RECORDING_DURATION_NS)
        {
            return Err(TerminalFailure::ResourceLimit);
        }
        Ok(())
    }

    fn record(
        &mut self,
        sequence: u64,
        timestamp: FrameTimestamp,
    ) -> Result<(), ScreenRecordingError> {
        self.submitted = self
            .submitted
            .checked_add(1)
            .ok_or(ScreenRecordingError::InvalidFrame)?;
        self.first_pts_ns.get_or_insert(timestamp.pts_ns);
        self.last_sequence = Some(sequence);
        self.last_end_pts_ns = Some(timestamp.end_ns());
        Ok(())
    }

    fn duration(self) -> Result<u64, ScreenRecordingError> {
        self.last_end_pts_ns
            .zip(self.first_pts_ns)
            .and_then(|(end, start)| end.checked_sub(start))
            .filter(|duration| *duration > 0)
            .ok_or(ScreenRecordingError::InvalidLifecycle)
    }
}

/// Single owner of a live BGRA + stereo F32LE appsrc-to-VP8/Opus WebM graph.
pub struct ScreenAudioRecording {
    pipeline: gst::Pipeline,
    video_appsrc: gst_app::AppSrc,
    audio_appsrc: gst_app::AppSrc,
    output: RecordingOutput,
    video_spec: ScreenRecordingSpec,
    audio_spec: SystemAudioRecordingSpec,
    state: RecordingState,
    video: SourceProgress,
    audio: SourceProgress,
    last_resource_check: Instant,
}

impl std::fmt::Debug for ScreenAudioRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenAudioRecording")
            .field("video_spec", &self.video_spec)
            .field("audio_spec", &self.audio_spec)
            .field("state", &self.state)
            .field("submitted_video_frames", &self.video.submitted)
            .field("submitted_audio_chunks", &self.audio.submitted)
            .field("output", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl ScreenAudioRecording {
    pub fn start(
        output_path: impl Into<PathBuf>,
        video_spec: ScreenRecordingSpec,
        audio_spec: SystemAudioRecordingSpec,
    ) -> Result<Self, ScreenRecordingError> {
        preflight_screen_audio_recording_runtime()?;
        let output = RecordingOutput::Managed(OutputReservation::for_filesink(output_path.into())?);
        Self::build(output, video_spec, audio_spec)
    }

    #[cfg(unix)]
    pub fn start_preopened(
        artifact_path: impl Into<PathBuf>,
        file: File,
        video_spec: ScreenRecordingSpec,
        audio_spec: SystemAudioRecordingSpec,
    ) -> Result<Self, ScreenRecordingError> {
        preflight_screen_audio_recording_runtime()?;
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
            video_spec,
            audio_spec,
        )
    }

    fn build(
        output: RecordingOutput,
        video_spec: ScreenRecordingSpec,
        audio_spec: SystemAudioRecordingSpec,
    ) -> Result<Self, ScreenRecordingError> {
        enforce_output_bounds(output.retained_file()?, true)?;
        #[cfg(unix)]
        let description = format!(
            concat!(
                "webmmux name=screen_audio_mux streamable=false ! fdsink name=screen_audio_sink sync=false ",
                "appsrc name=screen_audio_video_src ! queue max-size-buffers={} max-size-bytes={} max-size-time={} leaky=no ",
                "! videoconvert ! vp8enc deadline=1 ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 leaky=no ! screen_audio_mux. ",
                "appsrc name=screen_audio_audio_src ! queue max-size-buffers={} max-size-bytes={} max-size-time={} leaky=no ",
                "! audioconvert ! audioresample ! opusenc ! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 leaky=no ! screen_audio_mux."
            ),
            video_spec.ingress_max_frames(),
            SCREEN_RECORDING_QUEUE_BYTES,
            video_spec.ingress_max_time_ns(),
            AUDIO_QUEUE_BUFFERS,
            AUDIO_QUEUE_BYTES,
            AUDIO_QUEUE_TIME_NS,
        );
        #[cfg(not(unix))]
        let description = format!(
            concat!(
                "webmmux name=screen_audio_mux streamable=false ! filesink name=screen_audio_sink sync=false ",
                "appsrc name=screen_audio_video_src ! queue max-size-buffers={} max-size-bytes={} max-size-time={} leaky=no ",
                "! videoconvert ! vp8enc deadline=1 ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 leaky=no ! screen_audio_mux. ",
                "appsrc name=screen_audio_audio_src ! queue max-size-buffers={} max-size-bytes={} max-size-time={} leaky=no ",
                "! audioconvert ! audioresample ! opusenc ! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 leaky=no ! screen_audio_mux."
            ),
            video_spec.ingress_max_frames(),
            SCREEN_RECORDING_QUEUE_BYTES,
            video_spec.ingress_max_time_ns(),
            AUDIO_QUEUE_BUFFERS,
            AUDIO_QUEUE_BYTES,
            AUDIO_QUEUE_TIME_NS,
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|_| ScreenRecordingError::Pipeline)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| ScreenRecordingError::Pipeline)?;
        require_trusted(&pipeline)?;
        let video_appsrc = pipeline
            .by_name("screen_audio_video_src")
            .ok_or(ScreenRecordingError::Pipeline)?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| ScreenRecordingError::Pipeline)?;
        let audio_appsrc = pipeline
            .by_name("screen_audio_audio_src")
            .ok_or(ScreenRecordingError::Pipeline)?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| ScreenRecordingError::Pipeline)?;
        configure_video_appsrc(&video_appsrc, video_spec)?;
        configure_audio_appsrc(&audio_appsrc, audio_spec)?;
        let sink = pipeline
            .by_name("screen_audio_sink")
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
            video_appsrc,
            audio_appsrc,
            output,
            video_spec,
            audio_spec,
            state: RecordingState::Running,
            video: SourceProgress::new(),
            audio: SourceProgress::new(),
            last_resource_check: Instant::now(),
        })
    }

    pub fn push_video_frame(
        &mut self,
        frame: BgraScreenFrame,
    ) -> Result<ScreenAudioRecordingIngressStatus, ScreenRecordingError> {
        self.ensure_running()?;
        if u64::try_from(frame.byte_len()).ok() != Some(self.video_spec.frame_bytes()) {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        let sequence = frame.sequence();
        let timestamp = frame.timestamp();
        if let Err(failure) = self.video.validate_next(sequence, timestamp) {
            return self.fail(failure);
        }
        self.check_output_bounds()?;
        let (buffers, bytes, time_ns) = appsrc_levels(&self.video_appsrc);
        if buffers
            .checked_add(1)
            .is_none_or(|value| value > self.video_spec.ingress_max_frames())
            || bytes
                .checked_add(self.video_spec.frame_bytes())
                .is_none_or(|value| value > SCREEN_RECORDING_QUEUE_BYTES)
            || time_ns
                .checked_add(timestamp.duration_ns)
                .is_none_or(|value| value > self.video_spec.ingress_max_time_ns())
        {
            return self.fail(TerminalFailure::Backpressure);
        }
        let mut buffer = gst::Buffer::from_mut_slice(frame.pixels);
        if set_buffer_timing(&mut buffer, sequence, timestamp).is_err() {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        if self.video_appsrc.push_buffer(buffer).is_err() {
            return self.fail(TerminalFailure::Pipeline);
        }
        if self.video.record(sequence, timestamp).is_err() {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        Ok(self.status(
            &self.video_appsrc,
            self.video_spec.ingress_max_frames(),
            SCREEN_RECORDING_QUEUE_BYTES,
            self.video_spec.ingress_max_time_ns(),
        ))
    }

    pub fn push_audio_chunk(
        &mut self,
        chunk: F32StereoAudioChunk,
    ) -> Result<ScreenAudioRecordingIngressStatus, ScreenRecordingError> {
        self.ensure_running()?;
        let sequence = chunk.sequence;
        let timestamp = chunk.timestamp;
        let chunk_bytes =
            u64::try_from(chunk.samples.len()).map_err(|_| ScreenRecordingError::InvalidFrame)?;
        if chunk_bytes == 0 || chunk_bytes > MAX_AUDIO_CHUNK_BYTES {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        if let Err(failure) = self.audio.validate_next(sequence, timestamp) {
            return self.fail(failure);
        }
        self.check_output_bounds()?;
        let (buffers, bytes, time_ns) = appsrc_levels(&self.audio_appsrc);
        if buffers
            .checked_add(1)
            .is_none_or(|value| value > AUDIO_QUEUE_BUFFERS)
            || bytes
                .checked_add(chunk_bytes)
                .is_none_or(|value| value > AUDIO_QUEUE_BYTES)
            || time_ns
                .checked_add(timestamp.duration_ns)
                .is_none_or(|value| value > AUDIO_QUEUE_TIME_NS)
        {
            return self.fail(TerminalFailure::Backpressure);
        }
        let mut buffer = gst::Buffer::from_mut_slice(chunk.samples);
        if set_buffer_timing(&mut buffer, sequence, timestamp).is_err() {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        if self.audio_appsrc.push_buffer(buffer).is_err() {
            return self.fail(TerminalFailure::Pipeline);
        }
        if self.audio.record(sequence, timestamp).is_err() {
            return self.fail(TerminalFailure::InvalidFrame);
        }
        Ok(self.status(
            &self.audio_appsrc,
            AUDIO_QUEUE_BUFFERS,
            AUDIO_QUEUE_BYTES,
            AUDIO_QUEUE_TIME_NS,
        ))
    }

    pub fn end_of_stream(&mut self) -> Result<(), ScreenRecordingError> {
        match self.state {
            RecordingState::Running => {
                if self.pipeline_failed()
                    || self.video_appsrc.end_of_stream().is_err()
                    || self.audio_appsrc.end_of_stream().is_err()
                {
                    return self.fail(TerminalFailure::Pipeline);
                }
                self.state = RecordingState::EosSent;
                Ok(())
            }
            RecordingState::EosSent => Ok(()),
            RecordingState::Failed(failure) => Err(failure.error()),
        }
    }

    pub fn abort(self) -> Result<(), ScreenRecordingError> {
        set_null(&self.pipeline)
    }

    pub fn finish(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<ScreenAudioRecordingArtifact, ScreenRecordingError> {
        let lifecycle_error = match self.state {
            RecordingState::EosSent if self.video.submitted > 0 && self.audio.submitted > 0 => None,
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
        let expected_video = ExpectedVideo {
            frames: self.video.submitted,
            duration_ns: self.video.duration()?,
        };
        let expected_audio = ExpectedAudio {
            duration_ns: self.audio.duration()?,
        };
        #[cfg(unix)]
        let verified = verify_playable_av_webm_file(
            self.output.retained_file_mut()?,
            cancellation,
            expected_video,
            expected_audio,
        )?;
        #[cfg(not(unix))]
        let verified = verify_playable_av_webm(
            self.output.staging_path()?,
            cancellation,
            expected_video,
            expected_audio,
        )?;
        self.output.verify_ownership()?;
        enforce_output_bounds(self.output.retained_file()?, true)?;
        let path = self.output.commit()?;
        Ok(ScreenAudioRecordingArtifact {
            path,
            bytes: verified.bytes,
            sha256: verified.sha256,
            submitted_video_frames: self.video.submitted,
            encoded_video_frames: verified.encoded_video_frames,
            submitted_audio_chunks: self.audio.submitted,
            decoded_audio_buffers: verified.decoded_audio_buffers,
            video_duration_ns: verified.video_duration_ns,
            audio_duration_ns: verified.audio_duration_ns,
        })
    }

    fn ensure_running(&mut self) -> Result<(), ScreenRecordingError> {
        match self.state {
            RecordingState::Running => {}
            RecordingState::EosSent => return Err(ScreenRecordingError::InvalidLifecycle),
            RecordingState::Failed(failure) => return Err(failure.error()),
        }
        if self.pipeline_failed() {
            return self.fail(TerminalFailure::Pipeline);
        }
        Ok(())
    }

    fn check_output_bounds(&mut self) -> Result<(), ScreenRecordingError> {
        let check_free_space = self.last_resource_check.elapsed() >= RESOURCE_CHECK_INTERVAL;
        if enforce_output_bounds(self.output.retained_file()?, check_free_space).is_err() {
            return self.fail(TerminalFailure::ResourceLimit);
        }
        if check_free_space {
            self.last_resource_check = Instant::now();
        }
        Ok(())
    }

    fn status(
        &self,
        appsrc: &gst_app::AppSrc,
        maximum_buffers: u64,
        maximum_bytes: u64,
        maximum_time_ns: u64,
    ) -> ScreenAudioRecordingIngressStatus {
        let (queued_buffers, queued_bytes, queued_time_ns) = appsrc_levels(appsrc);
        ScreenAudioRecordingIngressStatus {
            submitted_video_frames: self.video.submitted,
            submitted_audio_chunks: self.audio.submitted,
            queued_buffers,
            queued_bytes,
            queued_time_ns,
            at_capacity: queued_buffers >= maximum_buffers
                || queued_bytes >= maximum_bytes
                || queued_time_ns >= maximum_time_ns,
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

impl Drop for ScreenAudioRecording {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

pub fn preflight_screen_audio_recording_runtime() -> Result<(), ScreenRecordingError> {
    prepare_runtime().map_err(ScreenRecordingError::Runtime)?;
    #[cfg(unix)]
    let sink = "fdsink";
    #[cfg(not(unix))]
    let sink = "filesink";
    for name in [
        "appsrc",
        "queue",
        "videoconvert",
        "vp8enc",
        "audioconvert",
        "audioresample",
        "opusenc",
        "webmmux",
        sink,
    ] {
        if gst::ElementFactory::find(name).is_none() {
            return Err(ScreenRecordingError::MissingFactory);
        }
    }
    preflight_av_verification()
}

fn configure_video_appsrc(
    appsrc: &gst_app::AppSrc,
    spec: ScreenRecordingSpec,
) -> Result<(), ScreenRecordingError> {
    let frame = spec.frame();
    let divisor = greatest_common_divisor(1_000_000_000, frame.nominal_frame_duration_ns);
    let numerator = i32::try_from(1_000_000_000 / divisor)
        .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
    let denominator = i32::try_from(frame.nominal_frame_duration_ns / divisor)
        .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
    appsrc.set_caps(Some(
        &gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("colorimetry", "sRGB")
            .field(
                "width",
                i32::try_from(frame.width)
                    .map_err(|_| ScreenRecordingError::InvalidConfiguration)?,
            )
            .field(
                "height",
                i32::try_from(frame.height)
                    .map_err(|_| ScreenRecordingError::InvalidConfiguration)?,
            )
            .field("framerate", gst::Fraction::new(numerator, denominator))
            .build(),
    ));
    configure_appsrc(
        appsrc,
        spec.ingress_max_frames(),
        SCREEN_RECORDING_QUEUE_BYTES,
        spec.ingress_max_time_ns(),
    );
    Ok(())
}

fn configure_audio_appsrc(
    appsrc: &gst_app::AppSrc,
    spec: SystemAudioRecordingSpec,
) -> Result<(), ScreenRecordingError> {
    let format = spec.format();
    let sample_rate = i32::try_from(format.sample_rate)
        .map_err(|_| ScreenRecordingError::InvalidConfiguration)?;
    appsrc.set_caps(Some(
        &gst::Caps::builder("audio/x-raw")
            .field("format", "F32LE")
            .field("layout", "interleaved")
            .field("rate", sample_rate)
            .field("channels", i32::from(format.channels))
            .build(),
    ));
    configure_appsrc(
        appsrc,
        AUDIO_QUEUE_BUFFERS,
        AUDIO_QUEUE_BYTES,
        AUDIO_QUEUE_TIME_NS,
    );
    Ok(())
}

fn configure_appsrc(
    appsrc: &gst_app::AppSrc,
    maximum_buffers: u64,
    maximum_bytes: u64,
    maximum_time_ns: u64,
) {
    appsrc.set_is_live(true);
    appsrc.set_format(gst::Format::Time);
    appsrc.set_do_timestamp(false);
    appsrc.set_block(false);
    appsrc.set_max_buffers(maximum_buffers);
    appsrc.set_max_bytes(maximum_bytes);
    appsrc.set_max_time(gst::ClockTime::from_nseconds(maximum_time_ns));
    appsrc.set_leaky_type(gst_app::AppLeakyType::None);
}

fn set_buffer_timing(
    buffer: &mut gst::Buffer,
    sequence: u64,
    timestamp: FrameTimestamp,
) -> Result<(), ScreenRecordingError> {
    let buffer = buffer.get_mut().ok_or(ScreenRecordingError::InvalidFrame)?;
    buffer.set_pts(gst::ClockTime::from_nseconds(timestamp.pts_ns));
    buffer.set_duration(gst::ClockTime::from_nseconds(timestamp.duration_ns));
    buffer.set_offset(sequence);
    buffer.set_offset_end(
        sequence
            .checked_add(1)
            .ok_or(ScreenRecordingError::InvalidFrame)?,
    );
    if timestamp.discontinuity {
        buffer.set_flags(gst::BufferFlags::DISCONT);
    }
    Ok(())
}

fn appsrc_levels(appsrc: &gst_app::AppSrc) -> (u64, u64, u64) {
    (
        appsrc.current_level_buffers(),
        appsrc.current_level_bytes(),
        appsrc.current_level_time().nseconds(),
    )
}

fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorSpace, FrameMemory, PixelFormat, VideoFrameSpec};
    use std::{thread, time::Duration};

    const TEST_FEED_TIMEOUT: Duration = Duration::from_secs(10);
    const TEST_FEED_POLL_INTERVAL: Duration = Duration::from_millis(1);

    #[derive(Clone, Copy)]
    struct TestIngressBudget {
        next_bytes: u64,
        next_duration_ns: u64,
        maximum_buffers: u64,
        maximum_bytes: u64,
        maximum_time_ns: u64,
        source: &'static str,
    }

    fn wait_for_test_ingress_capacity(
        appsrc: &gst_app::AppSrc,
        budget: TestIngressBudget,
        deadline: Instant,
    ) {
        loop {
            let (queued_buffers, queued_bytes, queued_time_ns) = appsrc_levels(appsrc);
            let has_capacity = queued_buffers
                .checked_add(1)
                .is_some_and(|value| value <= budget.maximum_buffers)
                && queued_bytes
                    .checked_add(budget.next_bytes)
                    .is_some_and(|value| value <= budget.maximum_bytes)
                && queued_time_ns
                    .checked_add(budget.next_duration_ns)
                    .is_some_and(|value| value <= budget.maximum_time_ns);
            if has_capacity {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {} ingress capacity: buffers={queued_buffers}, bytes={queued_bytes}, time_ns={queued_time_ns}",
                budget.source,
            );
            thread::sleep(TEST_FEED_POLL_INTERVAL);
        }
    }

    fn video_spec() -> ScreenRecordingSpec {
        ScreenRecordingSpec::new(VideoFrameSpec {
            width: 64,
            height: 36,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: FrameMemory::Cpu,
        })
        .expect("video spec")
    }

    fn audio_spec() -> SystemAudioRecordingSpec {
        SystemAudioRecordingSpec::new(AudioFormat {
            sample_rate: 48_000,
            channels: 2,
            sample_format: AudioSampleFormat::Float32,
        })
        .expect("audio spec")
    }

    fn audio_chunk(sequence: u64, pts_ns: u64) -> F32StereoAudioChunk {
        let samples = (0..480)
            .flat_map(|frame| {
                let value = ((frame as f32) / 480.0 * std::f32::consts::TAU).sin() * 0.1;
                [value.to_le_bytes(), value.to_le_bytes()].concat()
            })
            .collect();
        F32StereoAudioChunk::new(sequence, pts_ns, 10_000_000, false, samples).expect("audio chunk")
    }

    #[test]
    fn exact_audio_shape_and_finite_samples_are_required() {
        assert!(
            SystemAudioRecordingSpec::new(AudioFormat {
                sample_rate: 44_100,
                channels: 2,
                sample_format: AudioSampleFormat::Float32,
            })
            .is_err()
        );
        assert!(F32StereoAudioChunk::new(1, 0, 10_000_000, false, vec![0; 7]).is_err());
        assert!(
            F32StereoAudioChunk::new(
                1,
                0,
                20_833,
                false,
                f32::NAN
                    .to_le_bytes()
                    .into_iter()
                    .chain(0.0_f32.to_le_bytes())
                    .collect(),
            )
            .is_err()
        );
    }

    #[test]
    fn one_clock_graph_writes_independently_decodable_video_and_audio() {
        let directory = tempfile::tempdir().expect("output directory");
        let output = directory.path().join("screen-audio.webm");
        let video_spec = video_spec();
        let mut recording = ScreenAudioRecording::start(&output, video_spec, audio_spec())
            .expect("start A/V recording");
        let video_bytes = 64 * 36 * 4;
        let video_bytes_u64 = u64::try_from(video_bytes).expect("video byte count");
        let feed_deadline = Instant::now()
            .checked_add(TEST_FEED_TIMEOUT)
            .expect("bounded feeder deadline");
        let (mut video_sequence, mut audio_sequence) = (1_u64, 1_u64);
        while video_sequence <= 30 || audio_sequence <= 100 {
            let video_pts_ns = (video_sequence - 1) * 33_333_333;
            let audio_pts_ns = (audio_sequence - 1) * 10_000_000;
            if video_sequence <= 30 && (audio_sequence > 100 || video_pts_ns <= audio_pts_ns) {
                wait_for_test_ingress_capacity(
                    &recording.video_appsrc,
                    TestIngressBudget {
                        next_bytes: video_bytes_u64,
                        next_duration_ns: 33_333_333,
                        maximum_buffers: video_spec.ingress_max_frames(),
                        maximum_bytes: SCREEN_RECORDING_QUEUE_BYTES,
                        maximum_time_ns: video_spec.ingress_max_time_ns(),
                        source: "video",
                    },
                    feed_deadline,
                );
                recording
                    .push_video_frame(
                        BgraScreenFrame::new(
                            video_sequence,
                            FrameTimestamp {
                                pts_ns: video_pts_ns,
                                duration_ns: 33_333_333,
                                discontinuity: false,
                            },
                            vec![u8::try_from(video_sequence).unwrap_or(0); video_bytes],
                        )
                        .expect("video frame"),
                    )
                    .expect("push video");
                video_sequence += 1;
            } else {
                wait_for_test_ingress_capacity(
                    &recording.audio_appsrc,
                    TestIngressBudget {
                        next_bytes: 480 * AUDIO_BYTES_PER_FRAME,
                        next_duration_ns: 10_000_000,
                        maximum_buffers: AUDIO_QUEUE_BUFFERS,
                        maximum_bytes: AUDIO_QUEUE_BYTES,
                        maximum_time_ns: AUDIO_QUEUE_TIME_NS,
                        source: "audio",
                    },
                    feed_deadline,
                );
                recording
                    .push_audio_chunk(audio_chunk(audio_sequence, audio_pts_ns))
                    .expect("push audio");
                audio_sequence += 1;
            }
        }
        recording.end_of_stream().expect("EOS");
        let artifact = recording
            .finish(&CancellationToken::new())
            .expect("verified A/V artifact");
        assert_eq!(artifact.submitted_video_frames, 30);
        assert_eq!(artifact.encoded_video_frames, 30);
        assert_eq!(artifact.submitted_audio_chunks, 100);
        assert!(artifact.decoded_audio_buffers > 0);
        assert!(artifact.bytes > 128);
        assert_eq!(artifact.path, output);
    }

    #[test]
    fn invalid_ingress_is_terminal_and_cannot_publish_a_partial_timeline() {
        let directory = tempfile::tempdir().expect("output directory");
        let output = directory.path().join("invalid-screen-audio.webm");
        let mut recording = ScreenAudioRecording::start(&output, video_spec(), audio_spec())
            .expect("start A/V recording");
        let frame_bytes = 64 * 36 * 4;
        let frame = || {
            BgraScreenFrame::new(
                1,
                FrameTimestamp {
                    pts_ns: 0,
                    duration_ns: 33_333_333,
                    discontinuity: false,
                },
                vec![0; frame_bytes],
            )
            .expect("video frame")
        };
        recording.push_video_frame(frame()).expect("first frame");
        assert!(matches!(
            recording.push_video_frame(frame()),
            Err(ScreenRecordingError::NonMonotonicFrame)
        ));
        assert!(matches!(
            recording.end_of_stream(),
            Err(ScreenRecordingError::NonMonotonicFrame)
        ));
        assert!(matches!(
            recording.finish(&CancellationToken::new()),
            Err(ScreenRecordingError::NonMonotonicFrame)
        ));
        assert!(!output.exists());
    }
}
