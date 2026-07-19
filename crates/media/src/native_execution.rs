//! Production GStreamer execution paths for capture, Instant, and Studio.
//!
//! The contract modules deliberately contain no GStreamer objects. This module
//! is the native adapter at that boundary: descriptions contain only audited
//! factories and numeric contract values, paths are assigned as typed element
//! properties, queues are bounded, cancellation is polled, and every terminal
//! path confirms `Null` before returning.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AudioSampleFormat, AvFormat, AvPipelineGraphSpec, AvQueueSpec, AvSourceClass,
    CancellationToken, ExactCapsSpec, InstantAudioCaps, InstantPipelineRequest, InstantVideoCaps,
    PixelFormat, pipeline_has_trusted_factory_provenance, prepare_runtime,
};

const BUS_POLL: Duration = Duration::from_millis(25);
const AV_GRAPH_STATE_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(5);
const MIXED_AUDIO_SINK_MAX_BUFFERS: u32 = 128;
const CAMERA_RECORD_SINK_MAX_BUFFERS: u32 = 8;
const CAMERA_PREVIEW_SINK_MAX_BUFFERS: u32 = 2;
pub const MAX_AV_GRAPH_BUS_MESSAGES: u16 = 256;
const DEFAULT_NATIVE_DEADLINE: Duration = Duration::from_secs(120);
const MAX_NATIVE_SEGMENTS: usize = 100_000;
const MAX_PREVIEW_BYTES: usize = 1920 * 1080 * 4;
// Preview preroll performs real demuxing and decoding. Keep it bounded, while
// allowing a saturated production host (or the parallel GStreamer CI suite)
// enough time to schedule the asynchronous state transition.
const STUDIO_PREVIEW_STATE_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(30);

#[derive(Debug, Error)]
pub enum NativeExecutionError {
    #[error("native media graph is invalid")]
    InvalidGraph,
    #[error("native media graph has no source")]
    NoSources,
    #[error("required native media factory is unavailable")]
    MissingFactory,
    #[error("native media graph contains an undeclared or untrusted factory")]
    UntrustedFactory,
    #[error("native media graph failed")]
    Pipeline,
    #[error("native media graph timed out")]
    Timeout,
    #[error("native media graph was cancelled")]
    Cancelled,
    #[error("native media output is invalid")]
    InvalidOutput,
    #[error("native media output exceeded its bound")]
    ResourceLimit,
    #[error("native media filesystem operation failed")]
    Filesystem,
    #[error("H.264/AAC execution has not been explicitly approved")]
    CodecApprovalRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvGraphState {
    Null,
    Ready,
    Playing,
    EosRequested,
    EosObserved,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvGraphFailure {
    InvalidPollLimit,
    StateChange,
    BusUnavailable,
    PipelineError,
    EosRequest,
    NullNotReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvGraphTerminal {
    EndOfStream,
    Failed(NativeAvGraphFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAvGraphPollReport {
    pub messages_polled: u16,
    pub limit_reached: bool,
    pub terminal: Option<NativeAvGraphTerminal>,
}

/// A real A/V capture graph. Native bridges push master-corrected buffers into
/// the named appsrc elements; mixed audio and camera branches terminate at
/// bounded appsinks owned by the recording mode.
pub struct NativeAvGstreamerGraph {
    pipeline: gst::Pipeline,
    source_names: Vec<(AvSourceClass, &'static str)>,
    source_formats: Vec<(AvSourceClass, AvFormat)>,
    source_caps: Vec<(AvSourceClass, gst::Caps)>,
    source_ingress_budgets: Vec<(AvSourceClass, AvQueueSpec)>,
    queue_names: Vec<(AvSourceClass, &'static str)>,
    mixed_audio_sink: Option<&'static str>,
    camera_record_sink: Option<&'static str>,
    camera_preview_sink: Option<&'static str>,
    state: NativeAvGraphState,
}

impl std::fmt::Debug for NativeAvGstreamerGraph {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeAvGstreamerGraph")
            .field("sources", &self.source_names)
            .field("state", &self.state)
            .field("mixed_audio", &self.mixed_audio_sink.is_some())
            .field("camera_record", &self.camera_record_sink.is_some())
            .field("camera_preview", &self.camera_preview_sink.is_some())
            .finish_non_exhaustive()
    }
}

impl NativeAvGstreamerGraph {
    pub fn build(spec: &AvPipelineGraphSpec) -> Result<Self, NativeExecutionError> {
        prepare_runtime().map_err(|_| NativeExecutionError::MissingFactory)?;
        if spec.sources.is_empty() {
            return Err(NativeExecutionError::NoSources);
        }
        let mut description = String::new();
        let mut source_names = Vec::with_capacity(spec.sources.len());
        let mut source_formats = Vec::with_capacity(spec.sources.len());
        let mut source_ingress_budgets = Vec::with_capacity(spec.sources.len());
        let mut queue_names = Vec::with_capacity(spec.sources.len());
        let has_audio = spec.shared_audio_mixer.is_some();
        let sources_have_audio = spec.sources.iter().any(|source| {
            matches!(
                (source.class, source.input_caps),
                (
                    AvSourceClass::Microphone | AvSourceClass::SystemAudio,
                    ExactCapsSpec::Audio(_)
                )
            )
        });
        if has_audio != sources_have_audio {
            return Err(NativeExecutionError::InvalidGraph);
        }
        if has_audio {
            description.push_str(concat!(
                "audiomixer name=audio_mixer ignore-inactive-pads=false ",
                "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 leaky=downstream ",
                "! audioconvert ! audioresample ",
                "! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2 ",
                "! appsink name=mixed_audio_sink sync=false async=false wait-on-eos=false enable-last-sample=false max-buffers=128 drop=true "
            ));
        }
        for source in &spec.sources {
            match (source.class, source.input_caps, source.output_caps) {
                (
                    class @ (AvSourceClass::Microphone | AvSourceClass::SystemAudio),
                    ExactCapsSpec::Audio(input),
                    ExactCapsSpec::Audio(output),
                ) => {
                    let stages = source
                        .queue
                        .partition_ingress()
                        .map_err(|_| NativeExecutionError::InvalidGraph)?;
                    let name = match class {
                        AvSourceClass::Microphone => "microphone_src",
                        AvSourceClass::SystemAudio => "system_audio_src",
                        AvSourceClass::Camera => unreachable!(),
                    };
                    let queue_name = match class {
                        AvSourceClass::Microphone => "microphone_ingress_queue",
                        AvSourceClass::SystemAudio => "system_audio_ingress_queue",
                        AvSourceClass::Camera => unreachable!(),
                    };
                    source_names.push((class, name));
                    source_formats.push((class, AvFormat::Audio(input.format)));
                    source_ingress_budgets.push((class, source.queue));
                    queue_names.push((class, queue_name));
                    let input_caps = audio_caps(input.format, input.interleaved);
                    let output_caps = audio_caps(output.format, output.interleaved);
                    description.push_str(&format!(
                        "appsrc name={name} is-live=true do-timestamp=false block=false format=time \
                         max-buffers={} max-bytes={} max-time={} leaky-type=downstream caps=\"{input_caps}\" \
                         ! queue name={queue_name} max-size-buffers={} max-size-bytes={} max-size-time={} leaky=downstream \
                         ! audioconvert ! audioresample ! capsfilter caps=\"{output_caps}\" \
                         ! volume name={name}_volume ! level name={name}_level interval=100000000 post-messages=true \
                         ! audio_mixer. ",
                        stages.appsrc.max_buffers,
                        stages.appsrc.max_bytes,
                        stages.appsrc.max_age_ns,
                        stages.downstream.max_buffers,
                        stages.downstream.max_bytes,
                        stages.downstream.max_age_ns,
                    ));
                }
                (
                    AvSourceClass::Camera,
                    ExactCapsSpec::Camera(input),
                    ExactCapsSpec::Camera(output),
                ) => {
                    let stages = source
                        .queue
                        .partition_ingress()
                        .map_err(|_| NativeExecutionError::InvalidGraph)?;
                    source_names.push((AvSourceClass::Camera, "camera_src"));
                    source_formats.push((AvSourceClass::Camera, AvFormat::Camera(input.format)));
                    source_ingress_budgets.push((AvSourceClass::Camera, source.queue));
                    queue_names.push((AvSourceClass::Camera, "camera_ingress_queue"));
                    let input_caps = camera_caps(input.format);
                    let output_caps = camera_caps(output.format);
                    description.push_str(&format!(
                        "appsrc name=camera_src is-live=true do-timestamp=false block=false format=time \
                         max-buffers={} max-bytes={} max-time={} leaky-type=downstream caps=\"{input_caps}\" \
                         ! queue name=camera_ingress_queue max-size-buffers={} max-size-bytes={} max-size-time={} leaky=downstream \
                         ! videoconvert ! videoscale ! capsfilter caps=\"{output_caps}\" ! tee name=camera_tee \
                         camera_tee. ! queue max-size-buffers=8 max-size-bytes=134217728 max-size-time=500000000 leaky=downstream \
                         ! appsink name=camera_record_sink sync=false async=false wait-on-eos=false enable-last-sample=false max-buffers=8 drop=true ",
                        stages.appsrc.max_buffers,
                        stages.appsrc.max_bytes,
                        stages.appsrc.max_age_ns,
                        stages.downstream.max_buffers,
                        stages.downstream.max_bytes,
                        stages.downstream.max_age_ns,
                    ));
                    if spec.camera_preview_enabled {
                        description.push_str(concat!(
                            "camera_tee. ! queue max-size-buffers=2 max-size-bytes=33554432 max-size-time=200000000 leaky=downstream ",
                            "! videoconvert ! appsink name=camera_preview_sink sync=false async=false wait-on-eos=false enable-last-sample=false max-buffers=2 drop=true "
                        ));
                    }
                }
                _ => return Err(NativeExecutionError::InvalidGraph),
            }
        }
        let pipeline = parse_pipeline(&description)?;
        require_trusted(&pipeline)?;
        let mut source_caps = Vec::with_capacity(source_names.len());
        for (class, name) in &source_names {
            let appsrc = pipeline
                .by_name(name)
                .and_then(|element| element.downcast::<gst_app::AppSrc>().ok())
                .ok_or(NativeExecutionError::InvalidGraph)?;
            let caps = appsrc.caps().ok_or(NativeExecutionError::InvalidGraph)?;
            source_caps.push((*class, caps));
        }
        Ok(Self {
            pipeline,
            source_names,
            source_formats,
            source_caps,
            source_ingress_budgets,
            queue_names,
            mixed_audio_sink: has_audio.then_some("mixed_audio_sink"),
            camera_record_sink: spec
                .sources
                .iter()
                .any(|source| source.class == AvSourceClass::Camera)
                .then_some("camera_record_sink"),
            camera_preview_sink: spec.camera_preview_enabled.then_some("camera_preview_sink"),
            state: NativeAvGraphState::Null,
        })
    }

    #[must_use]
    pub fn pipeline(&self) -> &gst::Pipeline {
        &self.pipeline
    }

    pub fn source(&self, class: AvSourceClass) -> Option<gst::Element> {
        self.source_names
            .iter()
            .find(|(candidate, _)| *candidate == class)
            .and_then(|(_, name)| self.pipeline.by_name(name))
    }

    pub fn source_appsrc(&self, class: AvSourceClass) -> Option<gst_app::AppSrc> {
        self.source(class)?.downcast::<gst_app::AppSrc>().ok()
    }

    #[must_use]
    pub fn source_format(&self, class: AvSourceClass) -> Option<AvFormat> {
        self.source_formats
            .iter()
            .find_map(|(candidate, format)| (*candidate == class).then_some(*format))
    }

    /// Returns the authoritative aggregate budget from which the session,
    /// appsrc, and explicit queue stages were constructed.
    #[must_use]
    pub fn source_ingress_budget(&self, class: AvSourceClass) -> Option<AvQueueSpec> {
        self.source_ingress_budgets
            .iter()
            .find_map(|(candidate, budget)| (*candidate == class).then_some(*budget))
    }

    /// Revalidates mutable properties required for startup liveness. Before
    /// EOS is requested, the mixer must wait for every negotiated audio pad's
    /// first buffer instead of treating an all-inactive mixer as terminal.
    #[must_use]
    pub fn live_contract_is_intact(&self) -> bool {
        let source_caps_are_intact = self.source_caps.iter().all(|(class, expected)| {
            self.source_appsrc(*class)
                .and_then(|appsrc| appsrc.caps())
                .is_some_and(|actual| actual == *expected)
        });
        if !source_caps_are_intact {
            return false;
        }
        let sinks_are_nonblocking_at_eos = [
            (self.mixed_audio_sink, MIXED_AUDIO_SINK_MAX_BUFFERS),
            (self.camera_record_sink, CAMERA_RECORD_SINK_MAX_BUFFERS),
            (self.camera_preview_sink, CAMERA_PREVIEW_SINK_MAX_BUFFERS),
        ]
        .into_iter()
        .filter_map(|(name, max_buffers)| name.map(|name| (name, max_buffers)))
        .all(|(name, max_buffers)| {
            self.pipeline
                .by_name(name)
                .and_then(|sink| sink.downcast::<gst_app::AppSink>().ok())
                .is_some_and(|sink| {
                    !sink.property::<bool>("sync")
                        && !sink.property::<bool>("async")
                        && !sink.property::<bool>("enable-last-sample")
                        && !sink.is_wait_on_eos()
                        && sink.is_drop()
                        && sink.max_buffers() == max_buffers
                })
        });
        if !sinks_are_nonblocking_at_eos {
            return false;
        }
        if self.mixed_audio_sink.is_none() {
            return true;
        }
        self.pipeline.by_name("audio_mixer").is_some_and(|mixer| {
            mixer.find_property("force-live").is_some()
                && mixer.find_property("ignore-inactive-pads").is_some()
                && !mixer.property::<bool>("force-live")
                && !mixer.property::<bool>("ignore-inactive-pads")
        })
    }

    pub fn source_queue(&self, class: AvSourceClass) -> Option<gst::Element> {
        self.queue_names
            .iter()
            .find(|(candidate, _)| *candidate == class)
            .and_then(|(_, name)| self.pipeline.by_name(name))
    }

    #[must_use]
    pub const fn state(&self) -> NativeAvGraphState {
        self.state
    }

    pub fn mixed_audio_sink(&self) -> Option<gst::Element> {
        self.mixed_audio_sink
            .and_then(|name| self.pipeline.by_name(name))
    }

    pub fn camera_record_sink(&self) -> Option<gst::Element> {
        self.camera_record_sink
            .and_then(|name| self.pipeline.by_name(name))
    }

    pub fn camera_preview_sink(&self) -> Option<gst::Element> {
        self.camera_preview_sink
            .and_then(|name| self.pipeline.by_name(name))
    }

    pub fn prepare(&mut self) -> Result<(), NativeExecutionError> {
        if self.state != NativeAvGraphState::Null {
            return Err(NativeExecutionError::InvalidGraph);
        }
        self.pipeline
            .set_state(gst::State::Ready)
            .map_err(|_| NativeExecutionError::Pipeline)?;
        self.state = NativeAvGraphState::Ready;
        Ok(())
    }

    pub fn start_playing(&mut self) -> Result<(), NativeAvGraphFailure> {
        if !matches!(
            self.state,
            NativeAvGraphState::Null | NativeAvGraphState::Ready
        ) {
            return Err(NativeAvGraphFailure::StateChange);
        }
        if self.pipeline.set_state(gst::State::Playing).is_err() {
            self.state = NativeAvGraphState::Failed;
            return Err(NativeAvGraphFailure::StateChange);
        }
        let (transition, current, _) = self.pipeline.state(AV_GRAPH_STATE_TIMEOUT);
        if transition.is_err() || current != gst::State::Playing {
            self.state = NativeAvGraphState::Failed;
            return Err(NativeAvGraphFailure::StateChange);
        }
        self.state = NativeAvGraphState::Playing;
        Ok(())
    }

    pub fn request_eos(&mut self) -> Result<(), NativeAvGraphFailure> {
        if self.state != NativeAvGraphState::Playing {
            return Err(NativeAvGraphFailure::StateChange);
        }
        for (class, _) in &self.source_names {
            // GstBaseSrc controlled shutdown must enter through appsrc's
            // serialized event queue. That guarantees StreamStart, exact Caps,
            // and a TIME Segment precede EOS even when no buffer was pushed;
            // AppSrc::end_of_stream or direct pad events do not preserve this.
            let failed = self
                .source_appsrc(*class)
                .is_none_or(|source| !source.send_event(gst::event::Eos::new()));
            if failed {
                self.state = NativeAvGraphState::Failed;
                return Err(NativeAvGraphFailure::EosRequest);
            }
        }
        self.state = NativeAvGraphState::EosRequested;
        Ok(())
    }

    fn eos_stream_contract_is_intact(&self) -> bool {
        self.source_caps.iter().all(|(class, expected_caps)| {
            let Some(appsrc) = self.source_appsrc(*class) else {
                return false;
            };
            let Some(pad) = appsrc.static_pad("src") else {
                return false;
            };
            pad.sticky_event::<gst::event::StreamStart>(0).is_some()
                && pad
                    .sticky_event::<gst::event::Caps>(0)
                    .is_some_and(|event| event.caps() == expected_caps.as_ref())
                && pad
                    .sticky_event::<gst::event::Segment>(0)
                    .is_some_and(|event| event.segment().format() == gst::Format::Time)
        })
    }

    pub fn poll_bus(
        &mut self,
        max_messages: u16,
    ) -> Result<NativeAvGraphPollReport, NativeAvGraphFailure> {
        if max_messages == 0 || max_messages > MAX_AV_GRAPH_BUS_MESSAGES {
            return Err(NativeAvGraphFailure::InvalidPollLimit);
        }
        if !matches!(
            self.state,
            NativeAvGraphState::Playing | NativeAvGraphState::EosRequested
        ) {
            return Err(NativeAvGraphFailure::StateChange);
        }
        let Some(bus) = self.pipeline.bus() else {
            self.state = NativeAvGraphState::Failed;
            return Err(NativeAvGraphFailure::BusUnavailable);
        };
        let mut messages_polled = 0_u16;
        let mut terminal = None;
        while messages_polled < max_messages {
            let Some(message) = bus.pop() else {
                break;
            };
            messages_polled = messages_polled.saturating_add(1);
            match message.view() {
                gst::MessageView::Eos(_) => {
                    if self.eos_stream_contract_is_intact() {
                        self.state = NativeAvGraphState::EosObserved;
                        terminal = Some(NativeAvGraphTerminal::EndOfStream);
                    } else {
                        self.state = NativeAvGraphState::Failed;
                        terminal = Some(NativeAvGraphTerminal::Failed(
                            NativeAvGraphFailure::PipelineError,
                        ));
                    }
                    break;
                }
                gst::MessageView::Error(_) => {
                    self.state = NativeAvGraphState::Failed;
                    terminal = Some(NativeAvGraphTerminal::Failed(
                        NativeAvGraphFailure::PipelineError,
                    ));
                    break;
                }
                _ => {}
            }
        }
        Ok(NativeAvGraphPollReport {
            messages_polled,
            limit_reached: messages_polled == max_messages,
            terminal,
        })
    }

    pub fn confirm_null(&mut self) -> Result<(), NativeAvGraphFailure> {
        if self.pipeline.set_state(gst::State::Null).is_err() {
            self.state = NativeAvGraphState::Failed;
            return Err(NativeAvGraphFailure::StateChange);
        }
        let (transition, current, _) = self.pipeline.state(AV_GRAPH_STATE_TIMEOUT);
        if transition.is_err() || current != gst::State::Null {
            self.state = NativeAvGraphState::Failed;
            return Err(NativeAvGraphFailure::NullNotReached);
        }
        self.state = NativeAvGraphState::Null;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), NativeExecutionError> {
        self.confirm_null()
            .map_err(|_| NativeExecutionError::Pipeline)
    }
}

impl Drop for NativeAvGstreamerGraph {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn audio_caps(format: crate::AudioFormat, interleaved: bool) -> String {
    let sample = match format.sample_format {
        AudioSampleFormat::Signed16 => "S16LE",
        AudioSampleFormat::Signed32 => "S32LE",
        AudioSampleFormat::Float32 => "F32LE",
    };
    format!(
        "audio/x-raw,format={sample},layout={},rate={},channels={}",
        if interleaved {
            "interleaved"
        } else {
            "non-interleaved"
        },
        format.sample_rate,
        format.channels
    )
}

fn camera_caps(format: crate::CameraFormat) -> String {
    let pixel = match format.pixel_format {
        PixelFormat::Bgra8 => "BGRA",
        PixelFormat::Rgba8 => "RGBA",
        PixelFormat::Nv12 => "NV12",
        PixelFormat::I420 => "I420",
    };
    format!(
        "video/x-raw,format={pixel},width={},height={},framerate={}/{}",
        format.width, format.height, format.frame_rate_numerator, format.frame_rate_denominator
    )
}

/// One recovered, immutable fMP4 segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeInstantSegment {
    pub index: u32,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub has_file_type_box: bool,
    pub has_movie_or_fragment_box: bool,
}

/// Deterministic local manifest generated from the durable segment directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeInstantSegmentManifest {
    pub version: u16,
    pub segments: Vec<NativeInstantSegment>,
    pub total_bytes: u64,
}

impl NativeInstantSegmentManifest {
    pub fn recover(directory: &Path) -> Result<Self, NativeExecutionError> {
        let canonical =
            fs::canonicalize(directory).map_err(|_| NativeExecutionError::Filesystem)?;
        if !canonical.is_dir() {
            return Err(NativeExecutionError::InvalidOutput);
        }
        let mut paths = fs::read_dir(&canonical)
            .map_err(|_| NativeExecutionError::Filesystem)?
            .map(|entry| entry.map(|value| value.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| NativeExecutionError::Filesystem)?;
        paths.retain(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("segment-") && name.ends_with(".mp4"))
        });
        paths.sort();
        if paths.is_empty() || paths.len() > MAX_NATIVE_SEGMENTS {
            return Err(NativeExecutionError::InvalidOutput);
        }
        let mut segments = Vec::with_capacity(paths.len());
        let mut total_bytes = 0_u64;
        for (ordinal, path) in paths.into_iter().enumerate() {
            let expected = format!("segment-{ordinal:06}.mp4");
            if path.file_name().and_then(|value| value.to_str()) != Some(expected.as_str()) {
                return Err(NativeExecutionError::InvalidOutput);
            }
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| NativeExecutionError::Filesystem)?;
            if !metadata.file_type().is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() < 32
            {
                return Err(NativeExecutionError::InvalidOutput);
            }
            let mut file = File::open(&path).map_err(|_| NativeExecutionError::Filesystem)?;
            let mut prefix = vec![
                0_u8;
                usize::try_from(metadata.len().min(4 * 1024 * 1024))
                    .map_err(|_| NativeExecutionError::ResourceLimit)?
            ];
            file.read_exact(&mut prefix)
                .map_err(|_| NativeExecutionError::InvalidOutput)?;
            let has_file_type_box = prefix.windows(4).any(|value| value == b"ftyp");
            let has_movie_or_fragment_box = prefix
                .windows(4)
                .any(|value| matches!(value, b"moov" | b"moof"));
            if !has_file_type_box || !has_movie_or_fragment_box {
                return Err(NativeExecutionError::InvalidOutput);
            }
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or(NativeExecutionError::ResourceLimit)?;
            segments.push(NativeInstantSegment {
                index: u32::try_from(ordinal).map_err(|_| NativeExecutionError::ResourceLimit)?,
                sha256: sha256_file(&path)?,
                path,
                bytes: metadata.len(),
                has_file_type_box,
                has_movie_or_fragment_box,
            });
        }
        Ok(Self {
            version: 1,
            segments,
            total_bytes,
        })
    }
}

/// Execute a finite, real splitmux/mp4mux graph. This deterministic source is
/// used by conformance and recovery tests; production appsrc construction uses
/// [`build_instant_appsrc_pipeline`] below.
pub fn record_synthetic_instant_segments(
    directory: &Path,
    request: InstantPipelineRequest,
    duration: Duration,
    cancellation: &CancellationToken,
) -> Result<NativeInstantSegmentManifest, NativeExecutionError> {
    require_codec_approval()?;
    validate_instant_request(request)?;
    if duration.is_zero() || duration > Duration::from_secs(300) {
        return Err(NativeExecutionError::ResourceLimit);
    }
    create_private_directory(directory)?;
    let frames = duration_frames(request.video, duration)?;
    let audio_buffers = duration_audio_buffers(duration)?;
    let key_interval = duration_frames(
        request.video,
        Duration::from_nanos(request.segment_duration_ns),
    )?
    .max(1);
    let description = format!(
        concat!(
            "splitmuxsink name=segmenter async-finalize=false muxer-factory=mp4mux ",
            "max-size-time={} send-keyframe-requests=true ",
            "videotestsrc num-buffers={frames} is-live=false pattern=ball ",
            "! video/x-raw,format=I420,width={},height={},framerate={}/{} ",
            "! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ",
            "! videoconvert ! x264enc tune=zerolatency key-int-max={key_interval} byte-stream=false ",
            "! h264parse config-interval=-1 ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ! segmenter.video ",
            "audiotestsrc num-buffers={audio_buffers} samplesperbuffer=1024 is-live=false wave=sine freq=440 ",
            "! audio/x-raw,format=F32LE,rate=48000,channels=2 ",
            "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ",
            "! audioconvert ! audioresample ! avenc_aac ! aacparse ",
            "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ! segmenter.audio_0"
        ),
        request.segment_duration_ns,
        request.video.width,
        request.video.height,
        request.video.frame_rate_numerator,
        request.video.frame_rate_denominator,
        frames = frames,
        key_interval = key_interval,
        audio_buffers = audio_buffers,
    );
    let pipeline = parse_pipeline(&description)?;
    let segmenter = pipeline
        .by_name("segmenter")
        .ok_or(NativeExecutionError::InvalidGraph)?;
    segmenter.set_property(
        "location",
        directory
            .join("segment-%06d.mp4")
            .to_str()
            .ok_or(NativeExecutionError::Filesystem)?,
    );
    let fragment_millis = u32::try_from(request.segment_duration_ns / 1_000_000)
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    let muxer_properties = gst::Structure::builder("properties")
        .field("fragment-duration", fragment_millis)
        .field("streamable", true)
        .build();
    segmenter.set_property("muxer-properties", &muxer_properties);
    require_trusted(&pipeline)?;
    run_to_eos(&pipeline, cancellation, DEFAULT_NATIVE_DEADLINE)?;
    sync_directory(directory)?;
    NativeInstantSegmentManifest::recover(directory)
}

/// Build the production appsrc version of the Instant graph. Callers retain
/// typed handles to `instant_video_src` and `instant_audio_src` and push the
/// already master-clock-corrected buffers owned by the capture contracts.
pub fn build_instant_appsrc_pipeline(
    directory: &Path,
    request: InstantPipelineRequest,
) -> Result<gst::Pipeline, NativeExecutionError> {
    require_codec_approval()?;
    validate_instant_request(request)?;
    create_private_directory(directory)?;
    let key_interval = duration_frames(
        request.video,
        Duration::from_nanos(request.segment_duration_ns),
    )?
    .max(1);
    let audio = if let Some(caps) = request.audio {
        format!(
            concat!(
                "appsrc name=instant_audio_src is-live=true do-timestamp=false block=false format=time ",
                "caps=\"audio/x-raw,format=F32LE,layout=interleaved,rate={},channels={}\" ",
                "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 leaky=downstream ",
                "! audioconvert ! audioresample ! avenc_aac ! aacparse ",
                "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ! segmenter.audio_0 "
            ),
            caps.sample_rate, caps.channels
        )
    } else {
        String::new()
    };
    let description = format!(
        concat!(
            "splitmuxsink name=segmenter async-finalize=false muxer-factory=mp4mux max-size-time={} send-keyframe-requests=true ",
            "appsrc name=instant_video_src is-live=true do-timestamp=false block=false format=time ",
            "caps=\"video/x-raw,format=BGRA,width={},height={},framerate={}/{}\" ",
            "! queue max-size-buffers=16 max-size-bytes=134217728 max-size-time=500000000 leaky=downstream ",
            "! videoconvert ! x264enc tune=zerolatency key-int-max={key_interval} byte-stream=false ",
            "! h264parse config-interval=-1 ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ! segmenter.video ",
            "{audio}"
        ),
        request.segment_duration_ns,
        request.video.width,
        request.video.height,
        request.video.frame_rate_numerator,
        request.video.frame_rate_denominator,
        key_interval = key_interval,
        audio = audio,
    );
    let pipeline = parse_pipeline(&description)?;
    let segmenter = pipeline
        .by_name("segmenter")
        .ok_or(NativeExecutionError::InvalidGraph)?;
    segmenter.set_property(
        "location",
        directory
            .join("segment-%06d.mp4")
            .to_str()
            .ok_or(NativeExecutionError::Filesystem)?,
    );
    let fragment_millis = u32::try_from(request.segment_duration_ns / 1_000_000)
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    segmenter.set_property(
        "muxer-properties",
        gst::Structure::builder("properties")
            .field("fragment-duration", fragment_millis)
            .field("streamable", true)
            .build(),
    );
    require_trusted(&pipeline)?;
    Ok(pipeline)
}

fn validate_instant_request(request: InstantPipelineRequest) -> Result<(), NativeExecutionError> {
    if request.video.width == 0
        || request.video.height == 0
        || request.video.frame_rate_numerator == 0
        || request.video.frame_rate_denominator == 0
        || !(250_000_000..=30_000_000_000).contains(&request.segment_duration_ns)
        || request.max_split_slip_ns > request.segment_duration_ns / 4
        || request
            .audio
            .is_some_and(|audio| audio.sample_rate != 48_000 || audio.channels != 2)
    {
        return Err(NativeExecutionError::InvalidGraph);
    }
    Ok(())
}

fn duration_frames(
    video: InstantVideoCaps,
    duration: Duration,
) -> Result<u32, NativeExecutionError> {
    let numerator = duration
        .as_nanos()
        .checked_mul(u128::from(video.frame_rate_numerator))
        .ok_or(NativeExecutionError::ResourceLimit)?;
    let denominator = 1_000_000_000_u128
        .checked_mul(u128::from(video.frame_rate_denominator))
        .ok_or(NativeExecutionError::ResourceLimit)?;
    let frames = numerator
        .checked_add(denominator - 1)
        .and_then(|value| value.checked_div(denominator))
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(NativeExecutionError::ResourceLimit)?;
    Ok(frames)
}

fn duration_audio_buffers(duration: Duration) -> Result<u32, NativeExecutionError> {
    let samples = duration
        .as_nanos()
        .checked_mul(48_000)
        .and_then(|value| value.checked_add(999_999_999))
        .and_then(|value| value.checked_div(1_000_000_000))
        .ok_or(NativeExecutionError::ResourceLimit)?;
    u32::try_from(samples.div_ceil(1024)).map_err(|_| NativeExecutionError::ResourceLimit)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativeStudioTrackRole {
    Screen,
    Camera,
    Microphone,
    SystemAudio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioTrackArtifact {
    pub role: NativeStudioTrackRole,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// Record four isolated VP8/Opus WebM tracks on one GStreamer clock. This is
/// the executable conformance source; the appsrc production graph has the same
/// codecs and containers.
pub fn record_synthetic_studio_tracks(
    directory: &Path,
    duration: Duration,
    cancellation: &CancellationToken,
) -> Result<Vec<NativeStudioTrackArtifact>, NativeExecutionError> {
    if duration.is_zero() || duration > Duration::from_secs(300) {
        return Err(NativeExecutionError::ResourceLimit);
    }
    create_private_directory(directory)?;
    let frames = duration_frames(
        InstantVideoCaps {
            width: 320,
            height: 180,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
        },
        duration,
    )?;
    let audio_buffers = duration_audio_buffers(duration)?;
    let description = format!(
        concat!(
            "videotestsrc num-buffers={frames} is-live=false pattern=ball ",
            "! video/x-raw,width=320,height=180,framerate=30/1 ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ",
            "! videoconvert ! vp8enc deadline=1 ! webmmux name=screen_mux ! filesink name=screen_sink ",
            "videotestsrc num-buffers={frames} is-live=false pattern=smpte ",
            "! video/x-raw,width=320,height=180,framerate=30/1 ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ",
            "! videoconvert ! vp8enc deadline=1 ! webmmux name=camera_mux ! filesink name=camera_sink ",
            "audiotestsrc num-buffers={audio_buffers} samplesperbuffer=1024 is-live=false wave=sine freq=440 ",
            "! audio/x-raw,format=F32LE,rate=48000,channels=1 ! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ",
            "! audioconvert ! audioresample ! opusenc ! webmmux name=microphone_mux ! filesink name=microphone_sink ",
            "audiotestsrc num-buffers={audio_buffers} samplesperbuffer=1024 is-live=false wave=sine freq=880 ",
            "! audio/x-raw,format=F32LE,rate=48000,channels=2 ! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ",
            "! audioconvert ! audioresample ! opusenc ! webmmux name=system_audio_mux ! filesink name=system_audio_sink"
        ),
        frames = frames,
        audio_buffers = audio_buffers,
    );
    let pipeline = parse_pipeline(&description)?;
    let declarations = [
        (NativeStudioTrackRole::Screen, "screen_sink", "screen.webm"),
        (NativeStudioTrackRole::Camera, "camera_sink", "camera.webm"),
        (
            NativeStudioTrackRole::Microphone,
            "microphone_sink",
            "microphone.webm",
        ),
        (
            NativeStudioTrackRole::SystemAudio,
            "system_audio_sink",
            "system-audio.webm",
        ),
    ];
    for (_, sink, name) in declarations {
        pipeline
            .by_name(sink)
            .ok_or(NativeExecutionError::InvalidGraph)?
            .set_property("location", directory.join(name));
    }
    require_trusted(&pipeline)?;
    run_to_eos(&pipeline, cancellation, DEFAULT_NATIVE_DEADLINE)?;
    sync_directory(directory)?;
    declarations
        .into_iter()
        .map(|(role, _, name)| track_artifact(role, directory.join(name)))
        .collect()
}

/// Build the appsrc recording topology used by Studio capture. Each original
/// is encoded and committed independently; flattening cannot destroy editability.
pub fn build_studio_multitrack_appsrc_pipeline(
    directory: &Path,
    video: InstantVideoCaps,
    audio: InstantAudioCaps,
) -> Result<gst::Pipeline, NativeExecutionError> {
    if video.width == 0
        || video.height == 0
        || video.frame_rate_numerator == 0
        || video.frame_rate_denominator == 0
        || audio.sample_rate != 48_000
        || audio.channels == 0
    {
        return Err(NativeExecutionError::InvalidGraph);
    }
    create_private_directory(directory)?;
    let description = format!(
        concat!(
            "appsrc name=studio_screen_src is-live=true do-timestamp=false block=false format=time ",
            "caps=\"video/x-raw,format=BGRA,width={},height={},framerate={}/{}\" ",
            "! queue max-size-buffers=16 max-size-bytes=134217728 max-size-time=500000000 leaky=downstream ",
            "! videoconvert ! vp8enc deadline=1 ! webmmux ! filesink name=screen_sink ",
            "appsrc name=studio_camera_src is-live=true do-timestamp=false block=false format=time ",
            "caps=\"video/x-raw,format=BGRA,width={},height={},framerate={}/{}\" ",
            "! queue max-size-buffers=16 max-size-bytes=134217728 max-size-time=500000000 leaky=downstream ",
            "! videoconvert ! vp8enc deadline=1 ! webmmux ! filesink name=camera_sink ",
            "appsrc name=studio_microphone_src is-live=true do-timestamp=false block=false format=time ",
            "caps=\"audio/x-raw,format=F32LE,layout=interleaved,rate={},channels={}\" ",
            "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 leaky=downstream ",
            "! audioconvert ! audioresample ! opusenc ! webmmux ! filesink name=microphone_sink ",
            "appsrc name=studio_system_audio_src is-live=true do-timestamp=false block=false format=time ",
            "caps=\"audio/x-raw,format=F32LE,layout=interleaved,rate={},channels={}\" ",
            "! queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 leaky=downstream ",
            "! audioconvert ! audioresample ! opusenc ! webmmux ! filesink name=system_audio_sink"
        ),
        video.width,
        video.height,
        video.frame_rate_numerator,
        video.frame_rate_denominator,
        video.width,
        video.height,
        video.frame_rate_numerator,
        video.frame_rate_denominator,
        audio.sample_rate,
        audio.channels,
        audio.sample_rate,
        audio.channels,
    );
    let pipeline = parse_pipeline(&description)?;
    for (sink, name) in [
        ("screen_sink", "screen.webm"),
        ("camera_sink", "camera.webm"),
        ("microphone_sink", "microphone.webm"),
        ("system_audio_sink", "system-audio.webm"),
    ] {
        pipeline
            .by_name(sink)
            .ok_or(NativeExecutionError::InvalidGraph)?
            .set_property("location", directory.join(name));
    }
    require_trusted(&pipeline)?;
    Ok(pipeline)
}

fn track_artifact(
    role: NativeStudioTrackRole,
    path: PathBuf,
) -> Result<NativeStudioTrackArtifact, NativeExecutionError> {
    let metadata = fs::symlink_metadata(&path).map_err(|_| NativeExecutionError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() < 44 {
        return Err(NativeExecutionError::InvalidOutput);
    }
    let mut prefix = [0_u8; 4];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut prefix))
        .map_err(|_| NativeExecutionError::Filesystem)?;
    if prefix != [0x1a, 0x45, 0xdf, 0xa3] {
        return Err(NativeExecutionError::InvalidOutput);
    }
    Ok(NativeStudioTrackArtifact {
        role,
        sha256: sha256_file(&path)?,
        path,
        bytes: metadata.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioPreviewFrame {
    pub width: u32,
    pub height: u32,
    pub pts_ns: u64,
    pub rgb: Vec<u8>,
}

/// Decode one bounded RGB preview frame with playbin3 at the requested project
/// position. The bytes are real decoded media, not a mathematical seek record.
pub fn decode_studio_preview_frame(
    source: &Path,
    position: Duration,
    cancellation: &CancellationToken,
) -> Result<NativeStudioPreviewFrame, NativeExecutionError> {
    prepare_runtime().map_err(|_| NativeExecutionError::MissingFactory)?;
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    let canonical = fs::canonicalize(source).map_err(|_| NativeExecutionError::Filesystem)?;
    if !canonical.is_file() {
        return Err(NativeExecutionError::InvalidOutput);
    }
    let player = gst::ElementFactory::make("playbin3")
        .name("studio_preview_player")
        .build()
        .map_err(|_| NativeExecutionError::MissingFactory)?;
    let video_sink = gst::ElementFactory::make("appsink")
        .name("studio_preview_sink")
        .property("sync", false)
        .property("max-buffers", 1_u32)
        .property("drop", true)
        .build()
        .map_err(|_| NativeExecutionError::MissingFactory)?;
    video_sink.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("format", "RGB")
            .field("width", 320_i32)
            .field("height", 180_i32)
            .build(),
    );
    let audio_sink = gst::ElementFactory::make("fakesink")
        .property("sync", false)
        .build()
        .map_err(|_| NativeExecutionError::MissingFactory)?;
    player.set_property("video-sink", &video_sink);
    player.set_property("audio-sink", &audio_sink);
    let uri = gst::glib::filename_to_uri(&canonical, None)
        .map_err(|_| NativeExecutionError::Filesystem)?;
    player.set_property("uri", uri.as_str());
    let pipeline = gst::Pipeline::with_name("studio_preview_pipeline");
    pipeline
        .add(&player)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    require_trusted(&pipeline)?;
    pipeline
        .set_state(gst::State::Paused)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    let (transition, current, _) = pipeline.state(STUDIO_PREVIEW_STATE_TIMEOUT);
    if transition.is_err() || current != gst::State::Paused {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(NativeExecutionError::Pipeline);
    }
    if !pipeline_has_trusted_factory_provenance(&pipeline) {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(NativeExecutionError::UntrustedFactory);
    }
    if !position.is_zero()
        && player
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_nseconds(
                    u64::try_from(position.as_nanos())
                        .map_err(|_| NativeExecutionError::ResourceLimit)?,
                ),
            )
            .is_err()
    {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(NativeExecutionError::InvalidOutput);
    }
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    let sample = video_sink.emit_by_name::<Option<gst::Sample>>(
        "try-pull-sample",
        &[&gst::ClockTime::from_seconds(15)],
    );
    let result = (|| {
        if cancellation.is_cancelled() {
            return Err(NativeExecutionError::Cancelled);
        }
        let sample = sample.ok_or(NativeExecutionError::InvalidOutput)?;
        let caps = sample.caps().ok_or(NativeExecutionError::InvalidOutput)?;
        let structure = caps
            .structure(0)
            .ok_or(NativeExecutionError::InvalidOutput)?;
        let width = u32::try_from(
            structure
                .get::<i32>("width")
                .map_err(|_| NativeExecutionError::InvalidOutput)?,
        )
        .map_err(|_| NativeExecutionError::InvalidOutput)?;
        let height = u32::try_from(
            structure
                .get::<i32>("height")
                .map_err(|_| NativeExecutionError::InvalidOutput)?,
        )
        .map_err(|_| NativeExecutionError::InvalidOutput)?;
        let buffer = sample.buffer().ok_or(NativeExecutionError::InvalidOutput)?;
        let pts_ns = buffer.pts().map_or(0, gst::ClockTime::nseconds);
        let map = buffer
            .map_readable()
            .map_err(|_| NativeExecutionError::InvalidOutput)?;
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(3))
            .filter(|bytes| *bytes <= MAX_PREVIEW_BYTES)
            .ok_or(NativeExecutionError::ResourceLimit)?;
        if map.len() != expected {
            return Err(NativeExecutionError::InvalidOutput);
        }
        Ok(NativeStudioPreviewFrame {
            width,
            height,
            pts_ns,
            rgb: map.as_slice().to_vec(),
        })
    })();
    set_null(&pipeline)?;
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStudioExportProfile {
    EditableWebM,
    DistributionMasterMp4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioExportArtifact {
    pub profile: NativeStudioExportProfile,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub playable_container_marker: bool,
}

/// Decode and re-encode a playable Studio export. MP4 remains behind the
/// explicit codec decision; WebM is the fully executable local fallback.
pub fn render_studio_export(
    source: &Path,
    output: &Path,
    profile: NativeStudioExportProfile,
    cancellation: &CancellationToken,
) -> Result<NativeStudioExportArtifact, NativeExecutionError> {
    if profile == NativeStudioExportProfile::DistributionMasterMp4 {
        require_codec_approval()?;
    }
    let canonical = fs::canonicalize(source).map_err(|_| NativeExecutionError::Filesystem)?;
    if !canonical.is_file() || output.exists() {
        return Err(NativeExecutionError::InvalidOutput);
    }
    if let Some(parent) = output.parent() {
        create_private_directory(parent)?;
    }
    let (video, mux, marker) = match profile {
        NativeStudioExportProfile::EditableWebM => (
            "videoconvert ! vp8enc deadline=1",
            "webmmux",
            b"webm".as_slice(),
        ),
        NativeStudioExportProfile::DistributionMasterMp4 => (
            "videoconvert ! x264enc tune=zerolatency byte-stream=false ! h264parse config-interval=-1",
            "mp4mux faststart=true fragment-duration=2000 streamable=true",
            b"ftyp".as_slice(),
        ),
    };
    let description = format!(
        concat!(
            "filesrc name=source ! decodebin name=decode ",
            "{mux} name=export_mux ! filesink name=output ",
            "decode. ! queue max-size-buffers=64 max-size-bytes=134217728 max-size-time=2000000000 ",
            "! {video} ! queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ! export_mux."
        ),
        mux = mux,
        video = video,
    );
    let pipeline = parse_pipeline(&description)?;
    pipeline
        .by_name("source")
        .ok_or(NativeExecutionError::InvalidGraph)?
        .set_property("location", &canonical);
    pipeline
        .by_name("output")
        .ok_or(NativeExecutionError::InvalidGraph)?
        .set_property("location", output);
    require_trusted(&pipeline)?;
    let result = run_to_eos(&pipeline, cancellation, DEFAULT_NATIVE_DEADLINE);
    if result.is_err() {
        let _ = fs::remove_file(output);
        return result.map(|()| unreachable!());
    }
    let validated = (|| {
        let metadata =
            fs::symlink_metadata(output).map_err(|_| NativeExecutionError::Filesystem)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() < 128
        {
            return Err(NativeExecutionError::InvalidOutput);
        }
        let mut prefix = vec![
            0_u8;
            usize::try_from(metadata.len().min(1024 * 1024))
                .map_err(|_| NativeExecutionError::ResourceLimit)?
        ];
        File::open(output)
            .and_then(|mut file| file.read_exact(&mut prefix))
            .map_err(|_| NativeExecutionError::Filesystem)?;
        let playable_container_marker = prefix.windows(marker.len()).any(|value| value == marker);
        if !playable_container_marker {
            return Err(NativeExecutionError::InvalidOutput);
        }
        Ok(NativeStudioExportArtifact {
            profile,
            path: output.to_path_buf(),
            bytes: metadata.len(),
            sha256: sha256_file_with_budget(output, cancellation, DEFAULT_NATIVE_DEADLINE)?,
            playable_container_marker,
        })
    })();
    if validated.is_err() {
        let _ = fs::remove_file(output);
    }
    validated
}

fn parse_pipeline(description: &str) -> Result<gst::Pipeline, NativeExecutionError> {
    if description.is_empty() || description.len() > 64 * 1024 {
        return Err(NativeExecutionError::InvalidGraph);
    }
    prepare_runtime().map_err(|_| NativeExecutionError::MissingFactory)?;
    gst::parse::launch(description)
        .map_err(|_| NativeExecutionError::MissingFactory)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| NativeExecutionError::InvalidGraph)
}

fn require_trusted(pipeline: &gst::Pipeline) -> Result<(), NativeExecutionError> {
    if !pipeline_has_trusted_factory_provenance(pipeline) {
        return Err(NativeExecutionError::UntrustedFactory);
    }
    let names = pipeline
        .children()
        .iter()
        .filter_map(|element| element.factory().map(|factory| factory.name().to_string()))
        .collect::<BTreeSet<_>>();
    if names.is_empty() {
        return Err(NativeExecutionError::InvalidGraph);
    }
    Ok(())
}

fn run_to_eos(
    pipeline: &gst::Pipeline,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<(), NativeExecutionError> {
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    let bus = pipeline.bus().ok_or(NativeExecutionError::Pipeline)?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    let started = Instant::now();
    let terminal = loop {
        if cancellation.is_cancelled() {
            break Err(NativeExecutionError::Cancelled);
        }
        if started.elapsed() >= deadline {
            break Err(NativeExecutionError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL.as_millis() as u64),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            gst::MessageView::Error(_) => break Err(NativeExecutionError::Pipeline),
            _ => {}
        }
    };
    let teardown = set_null(pipeline);
    terminal?;
    teardown
}

pub(crate) fn set_null(pipeline: &gst::Pipeline) -> Result<(), NativeExecutionError> {
    pipeline
        .set_state(gst::State::Null)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    let (_, state, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    if state != gst::State::Null {
        return Err(NativeExecutionError::Pipeline);
    }
    Ok(())
}

pub(crate) fn require_codec_approval() -> Result<(), NativeExecutionError> {
    (std::env::var("FRAME_NATIVE_H264_AAC_APPROVED")
        .ok()
        .as_deref()
        == Some("approved-v1"))
    .then_some(())
    .ok_or(NativeExecutionError::CodecApprovalRequired)
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), NativeExecutionError> {
    fs::create_dir_all(path).map_err(|_| NativeExecutionError::Filesystem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| NativeExecutionError::Filesystem)?;
        let mode = fs::metadata(path)
            .map_err(|_| NativeExecutionError::Filesystem)?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(NativeExecutionError::Filesystem);
        }
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), NativeExecutionError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| NativeExecutionError::Filesystem)
}

fn sha256_file(path: &Path) -> Result<String, NativeExecutionError> {
    let mut file = File::open(path).map_err(|_| NativeExecutionError::Filesystem)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| NativeExecutionError::Filesystem)?;
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

pub(crate) fn sha256_file_with_budget(
    path: &Path,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<String, NativeExecutionError> {
    let started = Instant::now();
    let mut file = File::open(path).map_err(|_| NativeExecutionError::Filesystem)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_cancelled() {
            return Err(NativeExecutionError::Cancelled);
        }
        if started.elapsed() >= deadline {
            return Err(NativeExecutionError::Timeout);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|_| NativeExecutionError::Filesystem)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn native_av_spec() -> AvPipelineGraphSpec {
        let appsrc = crate::AvAppSrcBridgeSpec {
            is_live: true,
            do_timestamp: false,
            block: false,
            time_format_nanoseconds: true,
            timestamp_mode: crate::AppSrcTimestampMode::ExplicitMasterCorrected,
            retain_native_lease_until_downstream_release: true,
        };
        let audio_queue = crate::AvQueueSpec {
            max_buffers: 128,
            max_bytes: 8 * 1024 * 1024,
            max_age_ns: 2_000_000_000,
            backpressure: crate::AvBackpressurePolicy::DropOldest,
            producer_blocks: false,
        };
        let camera_queue = crate::AvQueueSpec {
            max_buffers: 8,
            max_bytes: 128 * 1024 * 1024,
            max_age_ns: 500_000_000,
            backpressure: crate::AvBackpressurePolicy::DropOldest,
            producer_blocks: false,
        };
        let mixed_audio = crate::AudioCapsSpec {
            format: crate::AudioFormat {
                sample_rate: 48_000,
                channels: 2,
                sample_format: AudioSampleFormat::Float32,
            },
            interleaved: true,
        };
        let camera_input = crate::CameraCapsSpec {
            format: crate::CameraFormat {
                width: 640,
                height: 360,
                frame_rate_numerator: 30,
                frame_rate_denominator: 1,
                pixel_format: PixelFormat::Bgra8,
            },
        };
        let camera_output = crate::CameraCapsSpec {
            format: crate::CameraFormat {
                pixel_format: PixelFormat::I420,
                ..camera_input.format
            },
        };
        let source = |class, marker, input_caps, output_caps, queue| crate::AvSourceGraphSpec {
            class,
            device: crate::AvDeviceId::from_opaque([marker; 16]).expect("device ID"),
            generation: crate::AvDeviceGeneration::new(1).expect("device generation"),
            input_caps,
            output_caps,
            appsrc,
            queue,
            elements: Vec::new(),
        };
        AvPipelineGraphSpec {
            sources: vec![
                source(
                    AvSourceClass::Microphone,
                    1,
                    ExactCapsSpec::Audio(crate::AudioCapsSpec {
                        format: crate::AudioFormat {
                            sample_rate: 44_100,
                            channels: 1,
                            sample_format: AudioSampleFormat::Signed16,
                        },
                        interleaved: true,
                    }),
                    ExactCapsSpec::Audio(mixed_audio),
                    audio_queue,
                ),
                source(
                    AvSourceClass::SystemAudio,
                    2,
                    ExactCapsSpec::Audio(mixed_audio),
                    ExactCapsSpec::Audio(mixed_audio),
                    audio_queue,
                ),
                source(
                    AvSourceClass::Camera,
                    3,
                    ExactCapsSpec::Camera(camera_input),
                    ExactCapsSpec::Camera(camera_output),
                    camera_queue,
                ),
            ],
            audio_mix_caps: mixed_audio,
            shared_audio_mixer: Some(crate::SharedAudioMixerTopology {
                element: crate::GstElementFamily::AudioMixer,
                request_pads: vec![
                    crate::AudioMixerRequestPadSpec {
                        class: AvSourceClass::Microphone,
                        pad: crate::AudioMixerPadId::Microphone,
                    },
                    crate::AudioMixerRequestPadSpec {
                        class: AvSourceClass::SystemAudio,
                        pad: crate::AudioMixerPadId::SystemAudio,
                    },
                ],
                output_caps: mixed_audio,
            }),
            camera_tee: Some(crate::CameraTeeTopology {
                element: crate::GstElementFamily::Tee,
                record_branch: vec![crate::GstElementFamily::Queue],
                preview_branch: Some(vec![
                    crate::GstElementFamily::Queue,
                    crate::GstElementFamily::VideoConvert,
                ]),
            }),
            camera_preview_enabled: true,
        }
    }

    fn native_av_stage_one_spec() -> AvPipelineGraphSpec {
        let mut spec = native_av_spec();
        for source in &mut spec.sources {
            source.queue.max_buffers = 3;
        }
        spec
    }

    #[test]
    fn native_av_graph_builds_real_mixer_resampler_and_camera_paths() {
        let mut graph = NativeAvGstreamerGraph::build(&native_av_spec()).expect("native A/V graph");
        assert!(graph.source(AvSourceClass::Microphone).is_some());
        assert!(graph.source(AvSourceClass::SystemAudio).is_some());
        assert!(graph.source(AvSourceClass::Camera).is_some());
        assert!(graph.mixed_audio_sink().is_some());
        assert!(graph.camera_record_sink().is_some());
        assert!(graph.camera_preview_sink().is_some());
        assert!(graph.live_contract_is_intact());

        for (sink, expected_max_buffers) in [
            (
                graph.mixed_audio_sink().expect("mixed audio sink"),
                MIXED_AUDIO_SINK_MAX_BUFFERS,
            ),
            (
                graph.camera_record_sink().expect("camera record sink"),
                CAMERA_RECORD_SINK_MAX_BUFFERS,
            ),
            (
                graph.camera_preview_sink().expect("camera preview sink"),
                CAMERA_PREVIEW_SINK_MAX_BUFFERS,
            ),
        ] {
            let sink = sink.downcast::<gst_app::AppSink>().expect("typed appsink");
            assert!(!sink.is_wait_on_eos());
            assert_eq!(sink.max_buffers(), expected_max_buffers);

            sink.set_wait_on_eos(true);
            assert!(!graph.live_contract_is_intact());
            sink.set_wait_on_eos(false);
            assert!(graph.live_contract_is_intact());

            sink.set_max_buffers(expected_max_buffers + 1);
            assert!(!graph.live_contract_is_intact());
            sink.set_max_buffers(expected_max_buffers);
            assert!(graph.live_contract_is_intact());

            sink.set_property("sync", true);
            assert!(!graph.live_contract_is_intact());
            sink.set_property("sync", false);
            assert!(graph.live_contract_is_intact());

            sink.set_property("async", true);
            assert!(!graph.live_contract_is_intact());
            sink.set_property("async", false);
            assert!(graph.live_contract_is_intact());

            sink.set_drop(false);
            assert!(!graph.live_contract_is_intact());
            sink.set_drop(true);
            assert!(graph.live_contract_is_intact());

            sink.set_property("enable-last-sample", true);
            assert!(!graph.live_contract_is_intact());
            sink.set_property("enable-last-sample", false);
            assert!(graph.live_contract_is_intact());
        }

        graph.prepare().expect("prepare native A/V graph");
        graph.stop().expect("stop native A/V graph");
    }

    #[test]
    fn native_av_graph_rejects_audio_sources_without_the_declared_shared_mixer() {
        let mut spec = native_av_spec();
        spec.shared_audio_mixer = None;

        assert!(matches!(
            NativeAvGstreamerGraph::build(&spec),
            Err(NativeExecutionError::InvalidGraph)
        ));
    }

    #[test]
    fn native_av_graph_immediate_eos_uses_time_segments_without_payload_primers() {
        let mut graph =
            NativeAvGstreamerGraph::build(&native_av_stage_one_spec()).expect("native A/V graph");
        graph.start_playing().expect("Playing state");
        for class in [
            AvSourceClass::Microphone,
            AvSourceClass::SystemAudio,
            AvSourceClass::Camera,
        ] {
            let appsrc = graph.source_appsrc(class).expect("typed appsrc");
            assert_eq!(appsrc.max_buffers(), 1);
            assert_eq!(appsrc.current_level_buffers(), 0);
            assert!(
                appsrc
                    .static_pad("src")
                    .expect("appsrc source pad")
                    .sticky_event::<gst::event::Segment>(0)
                    .is_none()
            );
        }

        graph.request_eos().expect("empty-source EOS");
        let deadline = Instant::now() + Duration::from_secs(5);
        let terminal = loop {
            let report = graph.poll_bus(32).expect("bounded bus poll");
            if let Some(terminal) = report.terminal {
                break Some(terminal);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(terminal, Some(NativeAvGraphTerminal::EndOfStream));

        for class in [
            AvSourceClass::Microphone,
            AvSourceClass::SystemAudio,
            AvSourceClass::Camera,
        ] {
            let appsrc = graph.source_appsrc(class).expect("typed appsrc");
            assert_eq!(appsrc.current_level_buffers(), 0);
            assert_eq!(
                appsrc
                    .static_pad("src")
                    .expect("appsrc source pad")
                    .sticky_event::<gst::event::Segment>(0)
                    .expect("serialized empty-source segment")
                    .segment()
                    .format(),
                gst::Format::Time
            );
        }
        // GstAggregator versions may materialize the empty serialized audio
        // interval as GAP buffers (some additionally mark them DROPPABLE).
        // Those carry no source payload and are distinct from forbidden fake
        // primer buffers.
        let mixed_audio_sink = graph
            .mixed_audio_sink()
            .expect("mixed audio sink")
            .downcast::<gst_app::AppSink>()
            .expect("typed mixed audio appsink");
        let mut gap_buffers = 0_u32;
        while let Some(sample) = mixed_audio_sink.try_pull_sample(gst::ClockTime::ZERO) {
            gap_buffers = gap_buffers.checked_add(1).expect("bounded gap count");
            assert!(gap_buffers <= MIXED_AUDIO_SINK_MAX_BUFFERS);
            let flags = sample.buffer().expect("gap buffer").flags();
            assert!(flags.contains(gst::BufferFlags::GAP));
        }
        for sink in [
            graph.camera_record_sink().expect("camera record sink"),
            graph.camera_preview_sink().expect("camera preview sink"),
        ] {
            let sink = sink.downcast::<gst_app::AppSink>().expect("typed appsink");
            assert!(sink.try_pull_sample(gst::ClockTime::ZERO).is_none());
        }
        graph.confirm_null().expect("confirmed Null");
    }

    #[test]
    fn native_av_graph_source_stage_preserves_one_real_buffer_before_serialized_eos() {
        let mut graph =
            NativeAvGstreamerGraph::build(&native_av_stage_one_spec()).expect("native A/V graph");
        graph.start_playing().expect("Playing state");
        let appsrc = graph
            .source_appsrc(AvSourceClass::SystemAudio)
            .expect("system audio appsrc");
        assert_eq!(appsrc.max_buffers(), 1);

        let observed_source_payloads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_by_probe = std::sync::Arc::clone(&observed_source_payloads);
        let source_pad = appsrc.static_pad("src").expect("appsrc source pad");
        let source_probe = source_pad
            .add_probe(gst::PadProbeType::BUFFER, move |_, information| {
                if let Some(gst::PadProbeData::Buffer(buffer)) = information.data.as_ref()
                    && let Ok(payload) = buffer.map_readable()
                {
                    observed_by_probe
                        .lock()
                        .expect("source probe lock")
                        .push(payload.as_slice().to_vec());
                }
                gst::PadProbeReturn::Ok
            })
            .expect("source buffer probe");

        // Use a non-silent valid stereo F32LE payload. GstAggregator versions
        // differ on whether the mixed output retains GAP when another mixer
        // pad reaches EOS without data, so flags alone cannot identify the
        // source-backed sample.
        let mut expected = Vec::with_capacity(8 * 480);
        for _ in 0..480 {
            expected.extend_from_slice(&0.25_f32.to_le_bytes());
            expected.extend_from_slice(&(-0.25_f32).to_le_bytes());
        }
        let mut buffer = gst::Buffer::from_slice(expected.clone());
        let buffer_ref = buffer.get_mut().expect("unique test buffer");
        buffer_ref.set_pts(gst::ClockTime::ZERO);
        buffer_ref.set_duration(gst::ClockTime::from_mseconds(10));
        appsrc.push_buffer(buffer).expect("one real buffer");
        assert!(appsrc.current_level_buffers() <= 1);

        graph.request_eos().expect("serialized source EOS");
        let deadline = Instant::now() + Duration::from_secs(5);
        let terminal = loop {
            let report = graph.poll_bus(32).expect("bounded bus poll");
            if let Some(terminal) = report.terminal {
                break Some(terminal);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(terminal, Some(NativeAvGraphTerminal::EndOfStream));
        assert_eq!(appsrc.current_level_buffers(), 0);

        let sink = graph
            .mixed_audio_sink()
            .and_then(|sink| sink.downcast::<gst_app::AppSink>().ok())
            .expect("mixed audio appsink");
        let mut sample_count = 0_u32;
        let mut mixed_source_payload_count = 0_u32;
        while let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) {
            sample_count = sample_count.checked_add(1).expect("bounded sample count");
            assert!(sample_count <= MIXED_AUDIO_SINK_MAX_BUFFERS);

            let buffer = sample.buffer().expect("sample buffer");
            let output = buffer.map_readable().expect("readable mixed output");
            if output.as_slice() == expected {
                mixed_source_payload_count = mixed_source_payload_count
                    .checked_add(1)
                    .expect("bounded mixed source payload count");
                assert_eq!(
                    mixed_source_payload_count, 1,
                    "only one source buffer was pushed"
                );
            } else {
                assert!(buffer.flags().contains(gst::BufferFlags::GAP));
            }
        }
        source_pad.remove_probe(source_probe);
        assert_eq!(
            *observed_source_payloads.lock().expect("source probe lock"),
            vec![expected],
            "serialized EOS must not overtake the accepted source buffer"
        );
        graph.confirm_null().expect("confirmed Null");
    }

    #[test]
    fn codec_profiles_fail_closed_without_explicit_approval() {
        // The test process intentionally does not mutate the approval setting.
        if std::env::var("FRAME_NATIVE_H264_AAC_APPROVED").is_err() {
            assert!(matches!(
                record_synthetic_instant_segments(
                    Path::new("/unused"),
                    InstantPipelineRequest {
                        video: InstantVideoCaps {
                            width: 320,
                            height: 180,
                            frame_rate_numerator: 30,
                            frame_rate_denominator: 1,
                        },
                        audio: Some(InstantAudioCaps {
                            sample_rate: 48_000,
                            channels: 2,
                        }),
                        segment_duration_ns: 1_000_000_000,
                        max_split_slip_ns: 100_000_000,
                    },
                    Duration::from_secs(2),
                    &CancellationToken::new(),
                ),
                Err(NativeExecutionError::CodecApprovalRequired)
            ));
        }
    }

    #[test]
    fn approved_instant_profile_writes_recoverable_fmp4_segments() {
        if std::env::var("FRAME_NATIVE_H264_AAC_APPROVED").as_deref() != Ok("approved-v1") {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let manifest = record_synthetic_instant_segments(
            directory.path(),
            InstantPipelineRequest {
                video: InstantVideoCaps {
                    width: 320,
                    height: 180,
                    frame_rate_numerator: 30,
                    frame_rate_denominator: 1,
                },
                audio: Some(InstantAudioCaps {
                    sample_rate: 48_000,
                    channels: 2,
                }),
                segment_duration_ns: 1_000_000_000,
                max_split_slip_ns: 100_000_000,
            },
            Duration::from_secs(3),
            &CancellationToken::new(),
        )
        .expect("recoverable fMP4 segments");
        assert!(manifest.segments.len() >= 2);
        assert!(manifest.total_bytes > 1_024);
        assert!(manifest.segments.iter().all(|segment| {
            segment.bytes > 128 && segment.has_file_type_box && segment.has_movie_or_fragment_box
        }));
    }

    #[test]
    fn studio_tracks_preview_and_webm_export_are_real_and_playable() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let tracks = record_synthetic_studio_tracks(
            directory.path(),
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .expect("real isolated tracks");
        assert_eq!(tracks.len(), 4);
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.role)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                NativeStudioTrackRole::Screen,
                NativeStudioTrackRole::Camera,
                NativeStudioTrackRole::Microphone,
                NativeStudioTrackRole::SystemAudio,
            ])
        );
        assert!(tracks.iter().all(|track| track.bytes > 1_024));

        let screen = tracks
            .iter()
            .find(|track| track.role == NativeStudioTrackRole::Screen)
            .expect("screen track");
        let preview = decode_studio_preview_frame(
            &screen.path,
            Duration::from_millis(500),
            &CancellationToken::new(),
        )
        .expect("decoded preview frame");
        assert_eq!((preview.width, preview.height), (320, 180));
        assert_eq!(preview.rgb.len(), 320 * 180 * 3);

        let export_path = directory.path().join("export.webm");
        let export = render_studio_export(
            &screen.path,
            &export_path,
            NativeStudioExportProfile::EditableWebM,
            &CancellationToken::new(),
        )
        .expect("playable WebM export");
        assert!(export.playable_container_marker);
        assert!(export.bytes > 1_024);
        let exported_preview = decode_studio_preview_frame(
            &export.path,
            Duration::from_millis(250),
            &CancellationToken::new(),
        )
        .expect("decode exported artifact");
        assert_eq!(
            (exported_preview.width, exported_preview.height),
            (320, 180)
        );

        if std::env::var("FRAME_NATIVE_H264_AAC_APPROVED").as_deref() == Ok("approved-v1") {
            let mp4_path = directory.path().join("distribution-master.mp4");
            let mp4 = render_studio_export(
                &screen.path,
                &mp4_path,
                NativeStudioExportProfile::DistributionMasterMp4,
                &CancellationToken::new(),
            )
            .expect("playable MP4 distribution master");
            assert!(mp4.playable_container_marker);
            assert!(mp4.bytes > 1_024);
            let probe = parse_pipeline(concat!(
                "filesrc name=probe_source ! qtdemux name=probe_demux ",
                "probe_demux.video_0 ! queue max-size-buffers=64 max-size-bytes=67108864 ",
                "max-size-time=2000000000 ! h264parse ! fakesink sync=false"
            ))
            .expect("MP4 demux probe");
            probe
                .by_name("probe_source")
                .expect("probe source")
                .set_property("location", &mp4.path);
            require_trusted(&probe).expect("trusted MP4 demux probe");
            run_to_eos(&probe, &CancellationToken::new(), DEFAULT_NATIVE_DEADLINE)
                .expect("demux and parse MP4 distribution master");
        }
    }
}
