//! Native isolated-track Studio recording.
//!
//! Each enabled source owns one bounded `appsrc` and one independent
//! VP8/Opus WebM branch. Encoded bytes are streamed directly into the durable
//! filesystem recording session, so an unwind or process loss leaves
//! checksum-recoverable per-track partials instead of a flattened artifact.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use thiserror::Error;

use crate::{
    AudioSampleFormat, CancellationToken, FilesystemStudioRecordingSession, FrameTimestamp,
    IsolatedTrackBranch, MAX_SCREEN_RECORDING_DURATION_NS, MAX_STUDIO_PAYLOAD_CHUNK_BYTES,
    NativeExecutionError, PixelFormat, RationalTime, StudioAsset, StudioAssetEncoding,
    StudioAssetRawCaps, StudioError, StudioRecordingGraphSpec, TrackKind,
    decode_studio_cursor_record, pipeline_has_trusted_factory_provenance, prepare_runtime,
    studio_cursor_timeline_header,
};

const BUS_POLL: Duration = Duration::from_millis(25);
const FINISH_TIMEOUT: Duration = Duration::from_secs(30);
const PIPELINE_STATE_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(5);

#[derive(Debug, Error)]
pub enum NativeStudioRecordingError {
    #[error("native Studio recording input is invalid")]
    InvalidInput,
    #[error("native Studio recording input is not monotonic")]
    NonMonotonicInput,
    #[error("native Studio recording ingress reached its bounded capacity")]
    Backpressure,
    #[error("native Studio recording lifecycle transition is invalid")]
    InvalidLifecycle,
    #[error(transparent)]
    Studio(#[from] StudioError),
    #[error(transparent)]
    Native(#[from] NativeExecutionError),
    #[error("native Studio recording failed and graph teardown also failed")]
    OperationAndTeardown {
        operation: Box<NativeStudioRecordingError>,
        teardown: Box<NativeStudioRecordingError>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStudioRecordingState {
    Running,
    EosRequested,
    Failed,
}

/// One owned, master-clock-corrected raw source buffer.
pub struct NativeStudioInputBuffer {
    track: TrackKind,
    sequence: u64,
    timestamp: FrameTimestamp,
    bytes: Vec<u8>,
}

impl NativeStudioInputBuffer {
    pub fn new(
        track: TrackKind,
        sequence: u64,
        timestamp: FrameTimestamp,
        bytes: Vec<u8>,
    ) -> Result<Self, NativeStudioRecordingError> {
        if sequence == 0
            || timestamp.duration_ns == 0
            || timestamp
                .pts_ns
                .checked_add(timestamp.duration_ns)
                .is_none()
            || bytes.is_empty()
        {
            return Err(NativeStudioRecordingError::InvalidInput);
        }
        Ok(Self {
            track,
            sequence,
            timestamp,
            bytes,
        })
    }

    #[must_use]
    pub const fn track(&self) -> TrackKind {
        self.track
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
        self.bytes.len()
    }
}

impl std::fmt::Debug for NativeStudioInputBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeStudioInputBuffer")
            .field("track", &self.track)
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field("retained_bytes", &self.bytes.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioTrackStatus {
    pub track: TrackKind,
    pub submitted_buffers: u64,
    pub encoded_bytes: u64,
    pub queued_buffers: u64,
    pub queued_bytes: u64,
    pub queued_time_ns: u64,
    pub at_capacity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioRecordingArtifact {
    pub assets: Vec<StudioAsset>,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub tracks: Vec<NativeStudioTrackStatus>,
}

#[derive(Debug, Clone, Copy)]
enum SourceFormat {
    Video {
        frame_bytes: u64,
    },
    Audio {
        bytes_per_frame: u64,
        sample_rate: u32,
    },
    Cursor {
        frame_width: u32,
        frame_height: u32,
    },
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

    fn validate(
        self,
        sequence: u64,
        timestamp: FrameTimestamp,
    ) -> Result<(), NativeStudioRecordingError> {
        if self
            .last_sequence
            .is_some_and(|previous| sequence <= previous)
            || self
                .last_end_pts_ns
                .is_some_and(|previous| timestamp.pts_ns < previous)
        {
            return Err(NativeStudioRecordingError::NonMonotonicInput);
        }
        let first = self.first_pts_ns.unwrap_or(timestamp.pts_ns);
        if timestamp
            .end_ns()
            .checked_sub(first)
            .is_none_or(|duration| duration > MAX_SCREEN_RECORDING_DURATION_NS)
        {
            return Err(NativeStudioRecordingError::InvalidInput);
        }
        Ok(())
    }

    fn record(
        &mut self,
        sequence: u64,
        timestamp: FrameTimestamp,
    ) -> Result<(), NativeStudioRecordingError> {
        self.submitted = self
            .submitted
            .checked_add(1)
            .ok_or(NativeStudioRecordingError::InvalidInput)?;
        self.first_pts_ns.get_or_insert(timestamp.pts_ns);
        self.last_sequence = Some(sequence);
        self.last_end_pts_ns = Some(timestamp.end_ns());
        Ok(())
    }
}

struct StudioSource {
    appsrc: gst_app::AppSrc,
    format: SourceFormat,
    maximum_buffers: u64,
    maximum_bytes: u64,
    maximum_time_ns: u64,
    encoded_bytes: Arc<AtomicU64>,
    progress: SourceProgress,
    cursor_revisions: BTreeSet<u64>,
    cursor_last_revision: Option<u64>,
}

/// One owner for the complete isolated-track recording graph.
pub struct NativeStudioRecording {
    pipeline: gst::Pipeline,
    sources: BTreeMap<TrackKind, StudioSource>,
    session: Arc<Mutex<Option<FilesystemStudioRecordingSession>>>,
    sink_failure: Arc<Mutex<Option<StudioError>>>,
    state: NativeStudioRecordingState,
}

impl std::fmt::Debug for NativeStudioRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeStudioRecording")
            .field("tracks", &self.sources.keys())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl NativeStudioRecording {
    pub fn start(
        graph: &StudioRecordingGraphSpec,
        session: FilesystemStudioRecordingSession,
    ) -> Result<Self, NativeStudioRecordingError> {
        graph.validate()?;
        if !session.can_start_native_encoding(graph) {
            return Err(NativeStudioRecordingError::InvalidLifecycle);
        }
        prepare_runtime().map_err(|_| NativeExecutionError::MissingFactory)?;

        let mut description = String::new();
        for branch in &graph.branches {
            let (source_name, sink_name) = element_names(branch.track);
            let encoder = match branch.track {
                TrackKind::Screen | TrackKind::Camera => "! videoconvert ! vp8enc deadline=1 ",
                TrackKind::Microphone | TrackKind::SystemAudio => {
                    "! audioconvert ! audioresample ! opusenc "
                }
                TrackKind::Cursor => "",
            };
            if branch.track == TrackKind::Cursor {
                description.push_str(&format!(
                    "appsrc name={source_name} ! appsink name={sink_name} "
                ));
            } else {
                description.push_str(&format!(
                    "appsrc name={source_name} {encoder}! webmmux streamable=true \
                     ! appsink name={sink_name} "
                ));
            }
        }
        let pipeline = gst::parse::launch(&description)
            .map_err(|_| NativeExecutionError::MissingFactory)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| NativeExecutionError::InvalidGraph)?;
        if !pipeline_has_trusted_factory_provenance(&pipeline) {
            return Err(NativeExecutionError::UntrustedFactory.into());
        }

        let session = Arc::new(Mutex::new(Some(session)));
        let sink_failure = Arc::new(Mutex::new(None));
        let mut sources = BTreeMap::new();
        for branch in &graph.branches {
            let (source_name, sink_name) = element_names(branch.track);
            let appsrc = pipeline
                .by_name(source_name)
                .and_then(|element| element.downcast::<gst_app::AppSrc>().ok())
                .ok_or(NativeExecutionError::InvalidGraph)?;
            let format = configure_source(&appsrc, branch)?;
            let sink = pipeline
                .by_name(sink_name)
                .and_then(|element| element.downcast::<gst_app::AppSink>().ok())
                .ok_or(NativeExecutionError::InvalidGraph)?;
            configure_sink(&sink, branch.queue.max_buffers);
            let encoded_bytes = Arc::new(AtomicU64::new(0));
            install_sink_callback(
                &sink,
                branch.track,
                Arc::clone(&session),
                Arc::clone(&sink_failure),
                Arc::clone(&encoded_bytes),
            );
            sources.insert(
                branch.track,
                StudioSource {
                    appsrc,
                    format,
                    maximum_buffers: u64::from(branch.queue.max_buffers),
                    maximum_bytes: branch.queue.max_bytes,
                    maximum_time_ns: branch.queue.max_time_ns,
                    encoded_bytes,
                    progress: SourceProgress::new(),
                    cursor_revisions: BTreeSet::new(),
                    cursor_last_revision: None,
                },
            );
        }
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| NativeExecutionError::Pipeline)?;
        let (transition, current, _) = pipeline.state(PIPELINE_STATE_TIMEOUT);
        if transition.is_err() || current != gst::State::Playing {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(NativeExecutionError::Pipeline.into());
        }
        for source in sources.values_mut() {
            let SourceFormat::Cursor {
                frame_width,
                frame_height,
            } = source.format
            else {
                continue;
            };
            let header = studio_cursor_timeline_header(frame_width, frame_height)
                .map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            source
                .appsrc
                .push_buffer(gst::Buffer::from_mut_slice(header))
                .map_err(|_| NativeExecutionError::Pipeline)?;
        }
        Ok(Self {
            pipeline,
            sources,
            session,
            sink_failure,
            state: NativeStudioRecordingState::Running,
        })
    }

    #[must_use]
    pub const fn state(&self) -> NativeStudioRecordingState {
        self.state
    }

    pub fn push(
        &mut self,
        input: NativeStudioInputBuffer,
    ) -> Result<NativeStudioTrackStatus, NativeStudioRecordingError> {
        let result = self.push_inner(input);
        if result.is_err() {
            self.state = NativeStudioRecordingState::Failed;
        }
        result
    }

    fn push_inner(
        &mut self,
        input: NativeStudioInputBuffer,
    ) -> Result<NativeStudioTrackStatus, NativeStudioRecordingError> {
        self.ensure_running()?;
        let earliest_pts_ns = self
            .sources
            .values()
            .filter_map(|source| source.progress.first_pts_ns)
            .min()
            .unwrap_or(input.timestamp.pts_ns)
            .min(input.timestamp.pts_ns);
        let latest_end_ns = self
            .sources
            .values()
            .filter_map(|source| source.progress.last_end_pts_ns)
            .max()
            .unwrap_or(input.timestamp.end_ns())
            .max(input.timestamp.end_ns());
        if latest_end_ns
            .checked_sub(earliest_pts_ns)
            .is_none_or(|duration| duration > MAX_SCREEN_RECORDING_DURATION_NS)
        {
            return Err(NativeStudioRecordingError::InvalidInput);
        }
        let source = self
            .sources
            .get_mut(&input.track)
            .ok_or(NativeStudioRecordingError::InvalidInput)?;
        validate_payload(source, input.sequence, input.timestamp, &input.bytes)?;
        source.progress.validate(input.sequence, input.timestamp)?;
        let (queued_buffers, queued_bytes, queued_time_ns) = source_levels(&source.appsrc);
        let retained_bytes = u64::try_from(input.bytes.len())
            .map_err(|_| NativeStudioRecordingError::InvalidInput)?;
        if queued_buffers
            .checked_add(1)
            .is_none_or(|value| value > source.maximum_buffers)
            || queued_bytes
                .checked_add(retained_bytes)
                .is_none_or(|value| value > source.maximum_bytes)
            || queued_time_ns
                .checked_add(input.timestamp.duration_ns)
                .is_none_or(|value| value > source.maximum_time_ns)
        {
            return Err(NativeStudioRecordingError::Backpressure);
        }
        let mut buffer = gst::Buffer::from_mut_slice(input.bytes);
        let writable = buffer
            .get_mut()
            .ok_or(NativeStudioRecordingError::InvalidInput)?;
        writable.set_pts(gst::ClockTime::from_nseconds(input.timestamp.pts_ns));
        writable.set_duration(gst::ClockTime::from_nseconds(input.timestamp.duration_ns));
        writable.set_offset(input.sequence);
        writable.set_offset_end(
            input
                .sequence
                .checked_add(1)
                .ok_or(NativeStudioRecordingError::InvalidInput)?,
        );
        if input.timestamp.discontinuity {
            writable.set_flags(gst::BufferFlags::DISCONT);
        }
        source
            .appsrc
            .push_buffer(buffer)
            .map_err(|_| NativeStudioRecordingError::Native(NativeExecutionError::Pipeline))?;
        source.progress.record(input.sequence, input.timestamp)?;
        Ok(track_status(input.track, source))
    }

    pub fn status(&self, track: TrackKind) -> Option<NativeStudioTrackStatus> {
        self.sources
            .get(&track)
            .map(|source| track_status(track, source))
    }

    pub fn end_of_stream(&mut self) -> Result<(), NativeStudioRecordingError> {
        let result = self.end_of_stream_inner();
        if result.is_err() {
            self.state = NativeStudioRecordingState::Failed;
        }
        result
    }

    fn end_of_stream_inner(&mut self) -> Result<(), NativeStudioRecordingError> {
        match self.state {
            NativeStudioRecordingState::Running => {
                if self
                    .sources
                    .values()
                    .any(|source| source.progress.submitted == 0)
                {
                    return Err(NativeStudioRecordingError::InvalidLifecycle);
                }
                for source in self.sources.values() {
                    source.appsrc.end_of_stream().map_err(|_| {
                        NativeStudioRecordingError::Native(NativeExecutionError::Pipeline)
                    })?;
                }
                self.state = NativeStudioRecordingState::EosRequested;
                Ok(())
            }
            NativeStudioRecordingState::EosRequested => Ok(()),
            NativeStudioRecordingState::Failed => Err(NativeStudioRecordingError::InvalidLifecycle),
        }
    }

    pub fn abort(self) -> Result<(), NativeStudioRecordingError> {
        set_null(&self.pipeline).map_err(Into::into)
    }

    pub fn finish(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<NativeStudioRecordingArtifact, NativeStudioRecordingError> {
        if self.state == NativeStudioRecordingState::Running {
            self.end_of_stream()?;
        }
        if self.state != NativeStudioRecordingState::EosRequested {
            return Err(NativeStudioRecordingError::InvalidLifecycle);
        }
        let terminal = wait_for_eos(&self.pipeline, cancellation);
        let teardown = set_null(&self.pipeline);
        match (terminal, teardown) {
            (Err(operation), Err(teardown)) => {
                return Err(NativeStudioRecordingError::OperationAndTeardown {
                    operation: Box::new(operation.into()),
                    teardown: Box::new(teardown.into()),
                });
            }
            (Err(operation), Ok(())) => return Err(operation.into()),
            (Ok(()), Err(teardown)) => return Err(teardown.into()),
            (Ok(()), Ok(())) => {}
        }
        if let Some(error) = take_sink_failure(&self.sink_failure)? {
            return Err(error.into());
        }
        let first_pts_ns = self
            .sources
            .values()
            .filter_map(|source| source.progress.first_pts_ns)
            .min()
            .ok_or(NativeStudioRecordingError::InvalidLifecycle)?;
        let end_pts_ns = self
            .sources
            .values()
            .filter_map(|source| source.progress.last_end_pts_ns)
            .max()
            .ok_or(NativeStudioRecordingError::InvalidLifecycle)?;
        let duration_ns = end_pts_ns
            .checked_sub(first_pts_ns)
            .filter(|duration| *duration > 0)
            .ok_or(NativeStudioRecordingError::InvalidInput)?;
        let start = RationalTime::from_nanos(first_pts_ns);
        let duration = RationalTime::from_nanos(duration_ns);
        let tracks = self
            .sources
            .iter()
            .map(|(track, source)| track_status(*track, source))
            .collect();
        let session = self
            .session
            .lock()
            .map_err(|_| NativeExecutionError::Pipeline)?
            .take()
            .ok_or(NativeStudioRecordingError::InvalidLifecycle)?;
        let assets = session.finish(start, duration)?;
        Ok(NativeStudioRecordingArtifact {
            assets,
            start,
            duration,
            tracks,
        })
    }

    fn ensure_running(&mut self) -> Result<(), NativeStudioRecordingError> {
        if self.state != NativeStudioRecordingState::Running {
            return Err(NativeStudioRecordingError::InvalidLifecycle);
        }
        if sink_failure(&self.sink_failure)?.is_some()
            || self
                .pipeline
                .bus()
                .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
                .is_some()
        {
            self.state = NativeStudioRecordingState::Failed;
            return Err(NativeExecutionError::Pipeline.into());
        }
        Ok(())
    }
}

impl Drop for NativeStudioRecording {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn element_names(track: TrackKind) -> (&'static str, &'static str) {
    match track {
        TrackKind::Screen => ("studio_screen_src", "studio_screen_sink"),
        TrackKind::Camera => ("studio_camera_src", "studio_camera_sink"),
        TrackKind::Microphone => ("studio_microphone_src", "studio_microphone_sink"),
        TrackKind::SystemAudio => ("studio_system_audio_src", "studio_system_audio_sink"),
        TrackKind::Cursor => ("studio_cursor_src", "studio_cursor_sink"),
    }
}

fn configure_source(
    appsrc: &gst_app::AppSrc,
    branch: &IsolatedTrackBranch,
) -> Result<SourceFormat, NativeStudioRecordingError> {
    let format = match branch.encoding {
        StudioAssetEncoding::Encoded {
            raw_caps: StudioAssetRawCaps::Video(caps),
            ..
        } => {
            if !matches!(branch.track, TrackKind::Screen | TrackKind::Camera)
                || caps.pixel_format != PixelFormat::Bgra8
            {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
            let width =
                i32::try_from(caps.width).map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            let height =
                i32::try_from(caps.height).map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            let numerator = i32::try_from(caps.frame_rate.numerator)
                .map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            let denominator = i32::try_from(caps.frame_rate.denominator)
                .map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            appsrc.set_caps(Some(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "BGRA")
                    .field("width", width)
                    .field("height", height)
                    .field("framerate", gst::Fraction::new(numerator, denominator))
                    .build(),
            ));
            let frame_bytes = u64::from(caps.width)
                .checked_mul(u64::from(caps.height))
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or(NativeStudioRecordingError::InvalidInput)?;
            SourceFormat::Video { frame_bytes }
        }
        StudioAssetEncoding::Encoded {
            raw_caps: StudioAssetRawCaps::Audio(caps),
            ..
        } => {
            if !matches!(branch.track, TrackKind::Microphone | TrackKind::SystemAudio)
                || caps.sample_rate != 48_000
                || caps.sample_format != AudioSampleFormat::Float32
            {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
            appsrc.set_caps(Some(
                &gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("layout", "interleaved")
                    .field(
                        "rate",
                        i32::try_from(caps.sample_rate)
                            .map_err(|_| NativeStudioRecordingError::InvalidInput)?,
                    )
                    .field("channels", i32::from(caps.channels))
                    .build(),
            ));
            SourceFormat::Audio {
                bytes_per_frame: u64::from(caps.channels) * 4,
                sample_rate: caps.sample_rate,
            }
        }
        StudioAssetEncoding::CursorTimelineV1 {
            frame_width,
            frame_height,
        } => {
            if branch.track != TrackKind::Cursor {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
            appsrc.set_caps(Some(
                &gst::Caps::builder("application/x-frame-cursor")
                    .field("version", 1_i32)
                    .field(
                        "frame-width",
                        i32::try_from(frame_width)
                            .map_err(|_| NativeStudioRecordingError::InvalidInput)?,
                    )
                    .field(
                        "frame-height",
                        i32::try_from(frame_height)
                            .map_err(|_| NativeStudioRecordingError::InvalidInput)?,
                    )
                    .build(),
            ));
            SourceFormat::Cursor {
                frame_width,
                frame_height,
            }
        }
        StudioAssetEncoding::UnspecifiedLegacyV1 => {
            return Err(NativeStudioRecordingError::InvalidInput);
        }
    };
    appsrc.set_is_live(true);
    appsrc.set_do_timestamp(false);
    appsrc.set_format(gst::Format::Time);
    appsrc.set_block(false);
    appsrc.set_max_buffers(u64::from(branch.queue.max_buffers));
    appsrc.set_max_bytes(branch.queue.max_bytes);
    appsrc.set_max_time(gst::ClockTime::from_nseconds(branch.queue.max_time_ns));
    appsrc.set_leaky_type(gst_app::AppLeakyType::None);
    Ok(format)
}

fn configure_sink(sink: &gst_app::AppSink, maximum_buffers: u32) {
    sink.set_property("sync", false);
    sink.set_property("async", false);
    sink.set_property("enable-last-sample", false);
    sink.set_max_buffers(maximum_buffers);
    sink.set_drop(false);
    sink.set_wait_on_eos(true);
}

fn install_sink_callback(
    sink: &gst_app::AppSink,
    track: TrackKind,
    session: Arc<Mutex<Option<FilesystemStudioRecordingSession>>>,
    sink_failure: Arc<Mutex<Option<StudioError>>>,
    encoded_bytes: Arc<AtomicU64>,
) {
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let mapped = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let bytes = mapped.as_slice();
                if bytes.is_empty() {
                    return Err(gst::FlowError::Error);
                }
                let result = session
                    .lock()
                    .map_err(|_| gst::FlowError::Error)?
                    .as_mut()
                    .ok_or(gst::FlowError::Error)
                    .and_then(|session| {
                        for chunk in bytes.chunks(MAX_STUDIO_PAYLOAD_CHUNK_BYTES) {
                            session.write_encoded_chunk(track, chunk).map_err(|error| {
                                if let Ok(mut failure) = sink_failure.lock()
                                    && failure.is_none()
                                {
                                    *failure = Some(error);
                                }
                                gst::FlowError::Error
                            })?;
                        }
                        Ok(())
                    });
                result?;
                encoded_bytes
                    .fetch_update(Ordering::Release, Ordering::Acquire, |current| {
                        current.checked_add(bytes.len() as u64)
                    })
                    .map_err(|_| gst::FlowError::Error)?;
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
}

fn validate_payload(
    source: &mut StudioSource,
    sequence: u64,
    timestamp: FrameTimestamp,
    bytes: &[u8],
) -> Result<(), NativeStudioRecordingError> {
    match source.format {
        SourceFormat::Video { frame_bytes } => {
            if u64::try_from(bytes.len()).ok() != Some(frame_bytes) {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
        }
        SourceFormat::Audio {
            bytes_per_frame,
            sample_rate,
        } => {
            let retained =
                u64::try_from(bytes.len()).map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            let frames = retained
                .checked_div(bytes_per_frame)
                .filter(|frames| {
                    *frames > 0 && frames.checked_mul(bytes_per_frame) == Some(retained)
                })
                .ok_or(NativeStudioRecordingError::InvalidInput)?;
            let expected_duration = frames
                .checked_mul(1_000_000_000)
                .and_then(|value| value.checked_div(u64::from(sample_rate)))
                .ok_or(NativeStudioRecordingError::InvalidInput)?;
            if timestamp.duration_ns != expected_duration
                || bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|sample| !f32::from_le_bytes(*sample).is_finite())
            {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
        }
        SourceFormat::Cursor {
            frame_width,
            frame_height,
        } => {
            let observation = decode_studio_cursor_record(bytes)
                .map_err(|_| NativeStudioRecordingError::InvalidInput)?;
            if observation.sequence() != sequence || observation.timestamp() != timestamp {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
            if observation
                .frame_position()
                .is_some_and(|(x, y)| x >= frame_width || y >= frame_height)
            {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
            if let Some(image) = observation.image_update() {
                if source
                    .cursor_last_revision
                    .is_some_and(|revision| image.revision() <= revision)
                    || !source.cursor_revisions.insert(image.revision())
                {
                    return Err(NativeStudioRecordingError::InvalidInput);
                }
                source.cursor_last_revision = Some(image.revision());
            }
            if observation
                .image_revision()
                .is_some_and(|revision| !source.cursor_revisions.contains(&revision))
            {
                return Err(NativeStudioRecordingError::InvalidInput);
            }
        }
    }
    Ok(())
}

fn source_levels(appsrc: &gst_app::AppSrc) -> (u64, u64, u64) {
    (
        appsrc.current_level_buffers(),
        appsrc.current_level_bytes(),
        appsrc.current_level_time().nseconds(),
    )
}

fn track_status(track: TrackKind, source: &StudioSource) -> NativeStudioTrackStatus {
    let (queued_buffers, queued_bytes, queued_time_ns) = source_levels(&source.appsrc);
    NativeStudioTrackStatus {
        track,
        submitted_buffers: source.progress.submitted,
        encoded_bytes: source.encoded_bytes.load(Ordering::Acquire),
        queued_buffers,
        queued_bytes,
        queued_time_ns,
        at_capacity: queued_buffers >= source.maximum_buffers
            || queued_bytes >= source.maximum_bytes
            || queued_time_ns >= source.maximum_time_ns,
    }
}

fn take_sink_failure(
    failure: &Mutex<Option<StudioError>>,
) -> Result<Option<StudioError>, NativeStudioRecordingError> {
    failure
        .lock()
        .map_err(|_| NativeExecutionError::Pipeline.into())
        .map(|mut failure| failure.take())
}

fn sink_failure(
    failure: &Mutex<Option<StudioError>>,
) -> Result<Option<StudioError>, NativeStudioRecordingError> {
    failure
        .lock()
        .map_err(|_| NativeExecutionError::Pipeline.into())
        .map(|failure| failure.clone())
}

fn wait_for_eos(
    pipeline: &gst::Pipeline,
    cancellation: &CancellationToken,
) -> Result<(), NativeExecutionError> {
    let bus = pipeline.bus().ok_or(NativeExecutionError::Pipeline)?;
    let started = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            return Err(NativeExecutionError::Cancelled);
        }
        if started.elapsed() >= FINISH_TIMEOUT {
            return Err(NativeExecutionError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL.as_millis() as u64),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => return Ok(()),
            gst::MessageView::Error(_) => return Err(NativeExecutionError::Pipeline),
            _ => {}
        }
    }
}

fn set_null(pipeline: &gst::Pipeline) -> Result<(), NativeExecutionError> {
    pipeline
        .set_state(gst::State::Null)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    let (_, state, _) = pipeline.state(PIPELINE_STATE_TIMEOUT);
    if state != gst::State::Null {
        return Err(NativeExecutionError::Pipeline);
    }
    Ok(())
}
