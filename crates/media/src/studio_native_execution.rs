//! Native preview and export adapters for [`StudioEditExecutor`].
//!
//! This is deliberately the smallest production slice that can execute rather
//! than merely describe an edit. It supports one required aligned screen
//! original plus independently optional aligned microphone and system-audio
//! originals. Temporal edits, rational speed, audio coverage gaps, gain, and
//! mute are applied from the shared executor. Camera composition, transformed
//! cursor metadata, nontransparent backgrounds, and camera-only/side-by-side
//! layouts fail closed until their native compositor is connected.

#[cfg(unix)]
use std::io::{Seek, SeekFrom};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::{
    collections::BTreeSet,
    fmt,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc::{Receiver, sync_channel},
    },
    time::{Duration, Instant},
};

use gst::prelude::*;
use gstreamer as gst;
use same_file::Handle;
use uuid::Uuid;

use crate::native_execution::{
    acquire_studio_native_execution_slot, create_private_directory, require_export_codec_approval,
    set_null, sha256_file_with_budget, sync_directory,
};
use crate::{
    BackgroundStyle, CancellationToken, CanonicalEditPlan, ExactDuration, ExportProfileSpec,
    FilesystemStudioOriginalStore, FrameRate, LayoutPreset, MAX_STUDIO_EDIT_EXECUTION_BATCH,
    MAX_STUDIO_EDIT_EXECUTION_WINDOWS, NativeExecutionError, NativeStudioExportProfile,
    NativeStudioPreviewFrame, RenderPhase, Sha256Digest, StudioAsset, StudioAssetId,
    StudioColorSpace, StudioEditExecutionError, StudioEditExecutionWindow, StudioEditExecutor,
    StudioPreviewGraphSpec, TrackKind, decode_studio_preview_frame,
    pipeline_has_only_declared_authored_factories, pipeline_has_trusted_factory_provenance,
    prepare_runtime,
};

const EDIT_EXECUTION_DEADLINE: Duration = Duration::from_secs(120);
const EDIT_STATE_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(10);
const EDIT_BUS_POLL: gst::ClockTime = gst::ClockTime::from_mseconds(25);
const MAX_EDIT_BUS_MESSAGES: usize = 100_000;
const MAX_EDIT_OUTPUT_PROBE_BYTES: u64 = 4 * 1024 * 1024;
const OUTPUT_RESERVATION_ATTEMPTS: usize = 8;
const MAX_STUDIO_ALIGNED_SOURCES: usize = 3;
const MAX_STUDIO_SEGMENT_BARRIER_BRANCHES: usize = MAX_STUDIO_ALIGNED_SOURCES;
const SEGMENT_COMPLETION_CHANNEL_CAPACITY: usize = MAX_STUDIO_SEGMENT_BARRIER_BRANCHES * 2;
const MAX_STUDIO_VIDEO_CLOSE_HOLD_NS: u64 = 1_000_000_000;

/// Originals whose timestamps share the canonical project clock. Optional
/// tracks must cover every executor window marked `source_available`; gaps are
/// represented by the plan and are rendered as silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioAlignedSources {
    pub screen: PathBuf,
    pub microphone: Option<PathBuf>,
    pub system_audio: Option<PathBuf>,
}

/// Caller-owned aligned originals opened before native export begins.
///
/// The renderer retains these descriptors for the entire GStreamer graph and
/// exposes them to the seekable file source only through `/dev/fd`; it never
/// resolves a project-controlled source pathname. This is intentionally
/// Unix-only because that descriptor namespace is the audited native desktop
/// boundary on those platforms.
#[cfg(unix)]
pub struct NativeStudioAlignedFileSources {
    pub screen: File,
    pub microphone: Option<File>,
    pub system_audio: Option<File>,
}

#[cfg(unix)]
impl fmt::Debug for NativeStudioAlignedFileSources {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioAlignedFileSources")
            .field("screen", &"<descriptor>")
            .field(
                "microphone",
                &self.microphone.as_ref().map(|_| "<descriptor>"),
            )
            .field(
                "system_audio",
                &self.system_audio.as_ref().map(|_| "<descriptor>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioEditedPreviewFrame {
    pub frame: NativeStudioPreviewFrame,
    pub requested_output_time: ExactDuration,
    pub resolved_source_time: ExactDuration,
    pub execution: StudioEditExecutionWindow,
    pub plan_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioEditedExportArtifact {
    pub profile: NativeStudioExportProfile,
    pub approved_profile: Option<ExportProfileSpec>,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub playable_container_marker: bool,
    pub audio_tracks: u8,
    pub video: NativeStudioOutputVideo,
    pub audio: Option<NativeStudioOutputAudio>,
    pub execution_windows: usize,
    pub plan_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioOutputVideo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub colorimetry: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioOutputAudio {
    pub sample_rate: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioRenderProgress {
    pub phase: RenderPhase,
    pub basis_points: u16,
    pub completed_windows: usize,
    pub total_windows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStudioPreviewPlaybackState {
    Paused,
    Playing,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioPreviewTrackPosition {
    pub asset_id: StudioAssetId,
    pub position: ExactDuration,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioPreviewVideoComponent {
    pub source: NativeStudioPreviewTrackPosition,
    pub frame: NativeStudioPreviewFrame,
}

impl fmt::Debug for NativeStudioPreviewVideoComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioPreviewVideoComponent")
            .field("source", &self.source)
            .field("width", &self.frame.width)
            .field("height", &self.frame.height)
            .field("pts_ns", &self.frame.pts_ns)
            .field("rgb_bytes", &self.frame.rgb.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioPreviewAudioDecision {
    pub source: Option<NativeStudioPreviewTrackPosition>,
    pub gain_millibels: i32,
    pub muted: bool,
}

impl NativeStudioPreviewAudioDecision {
    #[must_use]
    pub const fn silent(self) -> bool {
        self.source.is_none() || self.muted
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioPreviewSample {
    pub output_time: ExactDuration,
    pub source_time: ExactDuration,
    pub screen: NativeStudioPreviewVideoComponent,
    pub camera: Option<NativeStudioPreviewVideoComponent>,
    pub microphone: NativeStudioPreviewAudioDecision,
    pub system_audio: NativeStudioPreviewAudioDecision,
    pub execution: StudioEditExecutionWindow,
    pub plan_digest: Sha256Digest,
    pub source_set_digest: Sha256Digest,
    pub generation: u64,
    pub sequence: u64,
}

impl fmt::Debug for NativeStudioPreviewSample {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioPreviewSample")
            .field("output_time", &self.output_time)
            .field("source_time", &self.source_time)
            .field("screen", &self.screen)
            .field("camera", &self.camera)
            .field("microphone", &self.microphone)
            .field("system_audio", &self.system_audio)
            .field("execution", &self.execution)
            .field("plan_digest", &self.plan_digest)
            .field("source_set_digest", &self.source_set_digest)
            .field("generation", &self.generation)
            .field("sequence", &self.sequence)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioPreviewSnapshot {
    pub state: NativeStudioPreviewPlaybackState,
    pub playhead: ExactDuration,
    pub output_duration: ExactDuration,
    pub plan_digest: Sha256Digest,
    pub source_set_digest: Sha256Digest,
    pub generation: u64,
    pub next_sequence: u64,
}

struct NativeStudioPreviewAssetBinding {
    asset: StudioAsset,
    path: PathBuf,
    identity: Handle,
}

/// Stateful, bounded native preview owner for one immutable Studio source set.
///
/// The engine verifies every durable original through its filesystem sidecar,
/// resolves segmented assets at exact source positions, decodes real screen
/// and active camera frames, and emits exact audio source/gain/mute decisions
/// from the same executor used by export. It owns no unbounded frame queue:
/// every call returns at most one 320x180 RGB frame per active video track.
pub struct NativeStudioPreviewEngine {
    executor: StudioEditExecutor,
    source_set_digest: Sha256Digest,
    assets: Vec<NativeStudioPreviewAssetBinding>,
    frame_period: ExactDuration,
    mapped_vfr_points: Vec<ExactDuration>,
    state: NativeStudioPreviewPlaybackState,
    playhead: ExactDuration,
    generation: u64,
    next_sequence: u64,
}

impl fmt::Debug for NativeStudioPreviewEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioPreviewEngine")
            .field("plan_digest", &self.executor.plan_digest())
            .field("source_set_digest", &self.source_set_digest)
            .field("asset_count", &self.assets.len())
            .field("mapped_vfr_points", &self.mapped_vfr_points.len())
            .field("state", &self.state)
            .field("playhead", &self.playhead)
            .field("generation", &self.generation)
            .field("next_sequence", &self.next_sequence)
            .finish()
    }
}

impl NativeStudioPreviewEngine {
    pub fn open(
        graph: StudioPreviewGraphSpec,
        store: &mut FilesystemStudioOriginalStore,
        fallback_frame_rate: FrameRate,
        cancellation: &CancellationToken,
    ) -> Result<Self, NativeExecutionError> {
        if cancellation.is_cancelled() {
            return Err(NativeExecutionError::Cancelled);
        }
        graph
            .validate()
            .map_err(|_| NativeExecutionError::InvalidGraph)?;
        validate_preview_frame_rate(fallback_frame_rate)?;
        let executor = StudioEditExecutor::compile(
            &graph.plan,
            MAX_STUDIO_EDIT_EXECUTION_WINDOWS,
            cancellation,
        )
        .map_err(map_execution_error)?;
        let source_set_digest = graph.sources.digest();
        let project_id = graph.sources.project_id();
        let mut assets = Vec::with_capacity(graph.sources.assets().len());
        for asset in graph.sources.assets() {
            if cancellation.is_cancelled() {
                return Err(NativeExecutionError::Cancelled);
            }
            if asset.requires_encoding_probe() {
                return Err(NativeExecutionError::InvalidGraph);
            }
            let path = store
                .verified_original_path(project_id, asset)
                .map_err(|_| NativeExecutionError::InvalidOutput)?;
            let identity =
                Handle::from_path(&path).map_err(|_| NativeExecutionError::Filesystem)?;
            if assets
                .iter()
                .any(|binding: &NativeStudioPreviewAssetBinding| binding.identity == identity)
            {
                return Err(NativeExecutionError::InvalidGraph);
            }
            assets.push(NativeStudioPreviewAssetBinding {
                asset: asset.clone(),
                path,
                identity,
            });
        }
        let mapped_vfr_points = graph
            .plan
            .mapped_vfr_points(TrackKind::Screen)
            .map_err(|_| NativeExecutionError::InvalidGraph)?;
        let frame_period = ExactDuration::new(
            u128::from(fallback_frame_rate.denominator),
            u128::from(fallback_frame_rate.numerator),
        )
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
        Ok(Self {
            executor,
            source_set_digest,
            assets,
            frame_period,
            mapped_vfr_points,
            state: NativeStudioPreviewPlaybackState::Paused,
            playhead: ExactDuration::zero(),
            generation: 1,
            next_sequence: 1,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> NativeStudioPreviewSnapshot {
        NativeStudioPreviewSnapshot {
            state: self.state,
            playhead: self.playhead,
            output_duration: self.executor.output_duration(),
            plan_digest: self.executor.plan_digest(),
            source_set_digest: self.source_set_digest,
            generation: self.generation,
            next_sequence: self.next_sequence,
        }
    }

    pub fn play(&mut self) -> Result<(), NativeExecutionError> {
        match self.state {
            NativeStudioPreviewPlaybackState::Paused => {
                let generation = checked_counter(self.generation)?;
                self.state = NativeStudioPreviewPlaybackState::Playing;
                self.generation = generation;
                Ok(())
            }
            NativeStudioPreviewPlaybackState::Playing => Ok(()),
            NativeStudioPreviewPlaybackState::Ended => Err(NativeExecutionError::InvalidGraph),
        }
    }

    pub fn pause(&mut self) -> Result<(), NativeExecutionError> {
        if self.state == NativeStudioPreviewPlaybackState::Playing {
            let generation = checked_counter(self.generation)?;
            self.state = NativeStudioPreviewPlaybackState::Paused;
            self.generation = generation;
        }
        Ok(())
    }

    pub fn seek(
        &mut self,
        output_time: ExactDuration,
        cancellation: &CancellationToken,
    ) -> Result<NativeStudioPreviewSample, NativeExecutionError> {
        let generation = checked_counter(self.generation)?;
        let sequence = self.next_sequence;
        let next_sequence = checked_counter(sequence)?;
        let sample = self.sample_at(output_time, generation, sequence, cancellation)?;
        self.playhead = output_time;
        self.generation = generation;
        self.next_sequence = next_sequence;
        if self.state == NativeStudioPreviewPlaybackState::Ended {
            self.state = NativeStudioPreviewPlaybackState::Paused;
        }
        Ok(sample)
    }

    pub fn preview_current(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<NativeStudioPreviewSample, NativeExecutionError> {
        let sequence = self.next_sequence;
        let next_sequence = checked_counter(sequence)?;
        let sample = self.sample_at(self.playhead, self.generation, sequence, cancellation)?;
        self.next_sequence = next_sequence;
        Ok(sample)
    }

    pub fn next_frame(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<Option<NativeStudioPreviewSample>, NativeExecutionError> {
        match self.state {
            NativeStudioPreviewPlaybackState::Paused => {
                return Err(NativeExecutionError::InvalidGraph);
            }
            NativeStudioPreviewPlaybackState::Ended => return Ok(None),
            NativeStudioPreviewPlaybackState::Playing => {}
        }
        let next_playhead = self.next_playhead(self.playhead)?;
        let ended = next_playhead
            .compare(self.executor.output_duration())
            .map_err(|_| NativeExecutionError::ResourceLimit)?
            != std::cmp::Ordering::Less;
        let sequence = self.next_sequence;
        let next_sequence = checked_counter(sequence)?;
        let sample = self.sample_at(self.playhead, self.generation, sequence, cancellation)?;
        self.next_sequence = next_sequence;
        self.playhead = next_playhead;
        if ended {
            self.state = NativeStudioPreviewPlaybackState::Ended;
        }
        Ok(Some(sample))
    }

    fn sample_at(
        &self,
        output_time: ExactDuration,
        generation: u64,
        sequence: u64,
        cancellation: &CancellationToken,
    ) -> Result<NativeStudioPreviewSample, NativeExecutionError> {
        let execution = self
            .executor
            .window_at_output(output_time, cancellation)
            .map_err(map_execution_error)?;
        let source_time = execution
            .source_time_at_output(output_time)
            .map_err(map_execution_error)?;
        let screen = self.decode_video_component(TrackKind::Screen, source_time, cancellation)?;
        let camera = if execution.camera_source_available
            && execution.style.layout != LayoutPreset::ScreenOnly
        {
            Some(self.decode_video_component(TrackKind::Camera, source_time, cancellation)?)
        } else {
            None
        };
        let microphone = self.audio_decision(
            TrackKind::Microphone,
            source_time,
            execution.microphone.source_available,
            execution.microphone.style.gain_millibels,
            execution.microphone.style.muted,
        )?;
        let system_audio = self.audio_decision(
            TrackKind::SystemAudio,
            source_time,
            execution.system_audio.source_available,
            execution.system_audio.style.gain_millibels,
            execution.system_audio.style.muted,
        )?;
        Ok(NativeStudioPreviewSample {
            output_time,
            source_time,
            screen,
            camera,
            microphone,
            system_audio,
            execution,
            plan_digest: self.executor.plan_digest(),
            source_set_digest: self.source_set_digest,
            generation,
            sequence,
        })
    }

    fn decode_video_component(
        &self,
        track: TrackKind,
        source_time: ExactDuration,
        cancellation: &CancellationToken,
    ) -> Result<NativeStudioPreviewVideoComponent, NativeExecutionError> {
        let (binding, position) = self.binding_at(track, source_time)?;
        binding.verify_identity()?;
        let frame = decode_studio_preview_frame(
            &binding.path,
            exact_duration_to_std(position)?,
            cancellation,
        )?;
        Ok(NativeStudioPreviewVideoComponent {
            source: NativeStudioPreviewTrackPosition {
                asset_id: binding.asset.id,
                position,
            },
            frame,
        })
    }

    fn audio_decision(
        &self,
        track: TrackKind,
        source_time: ExactDuration,
        source_available: bool,
        gain_millibels: i32,
        muted: bool,
    ) -> Result<NativeStudioPreviewAudioDecision, NativeExecutionError> {
        let source = if source_available {
            let (binding, position) = self.binding_at(track, source_time)?;
            binding.verify_identity()?;
            Some(NativeStudioPreviewTrackPosition {
                asset_id: binding.asset.id,
                position,
            })
        } else {
            None
        };
        Ok(NativeStudioPreviewAudioDecision {
            source,
            gain_millibels,
            muted,
        })
    }

    fn binding_at(
        &self,
        track: TrackKind,
        source_time: ExactDuration,
    ) -> Result<(&NativeStudioPreviewAssetBinding, ExactDuration), NativeExecutionError> {
        let mut found = None;
        for binding in &self.assets {
            if binding.asset.track != track {
                continue;
            }
            let start = binding
                .asset
                .start
                .checked_sub(crate::RationalTime::new(0, binding.asset.start.time_base()))
                .map_err(|_| NativeExecutionError::InvalidGraph)?;
            let end = binding
                .asset
                .end()
                .map_err(|_| NativeExecutionError::InvalidGraph)?;
            if source_time
                .compare(start)
                .map_err(|_| NativeExecutionError::ResourceLimit)?
                != std::cmp::Ordering::Less
                && source_time
                    .compare(end)
                    .map_err(|_| NativeExecutionError::ResourceLimit)?
                    == std::cmp::Ordering::Less
            {
                if found.is_some() {
                    return Err(NativeExecutionError::InvalidGraph);
                }
                let position = source_time
                    .checked_sub(start)
                    .map_err(|_| NativeExecutionError::ResourceLimit)?;
                found = Some((binding, position));
            }
        }
        found.ok_or(NativeExecutionError::InvalidGraph)
    }

    fn next_playhead(&self, current: ExactDuration) -> Result<ExactDuration, NativeExecutionError> {
        let candidate = if self.mapped_vfr_points.is_empty() {
            current
                .checked_add(self.frame_period)
                .map_err(|_| NativeExecutionError::ResourceLimit)?
        } else {
            let mut low = 0_usize;
            let mut high = self.mapped_vfr_points.len();
            while low < high {
                let middle = low + (high - low) / 2;
                if self.mapped_vfr_points[middle]
                    .compare(current)
                    .map_err(|_| NativeExecutionError::ResourceLimit)?
                    == std::cmp::Ordering::Greater
                {
                    high = middle;
                } else {
                    low = middle.saturating_add(1);
                }
            }
            self.mapped_vfr_points
                .get(low)
                .copied()
                .unwrap_or_else(|| self.executor.output_duration())
        };
        if candidate
            .compare(self.executor.output_duration())
            .map_err(|_| NativeExecutionError::ResourceLimit)?
            == std::cmp::Ordering::Greater
        {
            Ok(self.executor.output_duration())
        } else {
            Ok(candidate)
        }
    }
}

impl NativeStudioPreviewAssetBinding {
    fn verify_identity(&self) -> Result<(), NativeExecutionError> {
        let metadata =
            fs::symlink_metadata(&self.path).map_err(|_| NativeExecutionError::Filesystem)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != self.asset.byte_len
            || Handle::from_path(&self.path).map_err(|_| NativeExecutionError::Filesystem)?
                != self.identity
        {
            return Err(NativeExecutionError::InvalidOutput);
        }
        Ok(())
    }
}

fn validate_preview_frame_rate(frame_rate: FrameRate) -> Result<(), NativeExecutionError> {
    frame_rate
        .validate()
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    let numerator = u64::from(frame_rate.numerator);
    let denominator = u64::from(frame_rate.denominator);
    if numerator < denominator
        || numerator
            > denominator
                .checked_mul(240)
                .ok_or(NativeExecutionError::ResourceLimit)?
    {
        return Err(NativeExecutionError::InvalidGraph);
    }
    Ok(())
}

fn checked_counter(value: u64) -> Result<u64, NativeExecutionError> {
    value
        .checked_add(1)
        .ok_or(NativeExecutionError::ResourceLimit)
}

/// Decode a real screen frame at the source position selected by the shared
/// edit executor. The returned execution window is the composition/audio input
/// for the preview compositor; this function does not claim to draw it.
pub fn decode_studio_edited_preview_frame(
    screen: &Path,
    plan: &CanonicalEditPlan,
    output_time: ExactDuration,
    cancellation: &CancellationToken,
) -> Result<NativeStudioEditedPreviewFrame, NativeExecutionError> {
    let executor =
        StudioEditExecutor::compile(plan, MAX_STUDIO_EDIT_EXECUTION_WINDOWS, cancellation)
            .map_err(map_execution_error)?;
    let execution = executor
        .window_at_output(output_time, cancellation)
        .map_err(map_execution_error)?;
    let source_time = execution
        .source_time_at_output(output_time)
        .map_err(map_execution_error)?;
    let frame =
        decode_studio_preview_frame(screen, exact_duration_to_std(source_time)?, cancellation)?;
    Ok(NativeStudioEditedPreviewFrame {
        frame,
        requested_output_time: output_time,
        resolved_source_time: source_time,
        execution,
        plan_digest: executor.plan_digest(),
    })
}

/// Render a playable edit-aware export from aligned isolated originals.
///
/// Every timeline window is executed as an accurate bounded segment seek. An
/// `identity single-segment` stage converts each source segment to one
/// continuous output timeline; audio branches are mixed only after the same
/// mapping and receive the executor's exact gap/gain/mute state.
pub fn render_studio_export_with_edits(
    sources: &NativeStudioAlignedSources,
    output: &Path,
    plan: &CanonicalEditPlan,
    profile: NativeStudioExportProfile,
    cancellation: &CancellationToken,
) -> Result<NativeStudioEditedExportArtifact, NativeExecutionError> {
    render_studio_export_with_edits_and_progress(
        sources,
        output,
        plan,
        profile,
        cancellation,
        |_| {},
    )
}

/// Render an edit-aware artifact while publishing a bounded monotonic progress
/// trace. The callback is synchronous and receives at most one event per
/// execution window plus six lifecycle events; it owns no media bytes.
pub fn render_studio_export_with_edits_and_progress(
    sources: &NativeStudioAlignedSources,
    output: &Path,
    plan: &CanonicalEditPlan,
    profile: NativeStudioExportProfile,
    cancellation: &CancellationToken,
    progress: impl FnMut(NativeStudioRenderProgress),
) -> Result<NativeStudioEditedExportArtifact, NativeExecutionError> {
    let _studio_slot = acquire_studio_native_execution_slot(cancellation)?;
    require_export_codec_approval(profile)?;
    let mut executor =
        StudioEditExecutor::compile(plan, MAX_STUDIO_EDIT_EXECUTION_WINDOWS, cancellation)
            .map_err(map_execution_error)?;
    validate_supported_composition(executor.windows(), sources)?;
    let approved_profile = profile
        .approved_profile()
        .map(ExportProfileSpec::approved)
        .map(ExportProfileSpec::validate)
        .transpose()
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    if approved_profile.is_some() && sources.microphone.is_none() && sources.system_audio.is_none()
    {
        return Err(NativeExecutionError::InvalidGraph);
    }
    let total_windows = executor.windows().len();
    let mut progress = NativeStudioProgressEmitter::new(progress, total_windows);
    progress.emit(RenderPhase::Preparing, 0, 0)?;
    let canonical = canonicalize_sources(sources)?;
    let mut reservation = StudioOutputReservation::new(output.to_path_buf(), profile)?;

    let pipeline = build_edit_pipeline(
        &canonical,
        StudioEditOutput::Path(reservation.staging_path()),
        profile,
    )?;
    let result = execute_windows(
        &pipeline,
        &mut executor,
        &canonical,
        profile,
        cancellation,
        &mut progress,
    );
    if result.is_err() {
        let _ = set_null(&pipeline);
        return result.map(|()| unreachable!());
    }
    set_null(&pipeline)?;
    progress.emit(RenderPhase::Finalizing, 9_800, total_windows)?;
    reservation.adopt_created()?;
    let mut artifact = validate_edited_output(
        reservation.staging_path(),
        profile,
        approved_profile,
        canonical.audio_track_count(),
        executor.windows().len(),
        executor.plan_digest(),
        cancellation,
    )?;
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    artifact.path = reservation.commit()?;
    progress.emit(RenderPhase::Finalizing, 10_000, total_windows)?;
    Ok(artifact)
}

/// Render into a caller-owned, already-opened empty regular file.
///
/// Unix desktop adapters use this boundary to keep every decoded and encoded
/// byte attached to caller-owned source and staging inodes. `artifact_path` is
/// presentation metadata only and is never opened by this function.
#[cfg(unix)]
pub fn render_studio_export_with_edits_preopened(
    mut sources: NativeStudioAlignedFileSources,
    artifact_path: &Path,
    mut output: File,
    plan: &CanonicalEditPlan,
    profile: NativeStudioExportProfile,
    cancellation: &CancellationToken,
) -> Result<NativeStudioEditedExportArtifact, NativeExecutionError> {
    let _studio_slot = acquire_studio_native_execution_slot(cancellation)?;
    require_export_codec_approval(profile)?;
    let metadata = output
        .metadata()
        .map_err(|_| NativeExecutionError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.len() != 0 {
        return Err(NativeExecutionError::InvalidOutput);
    }
    let mut executor =
        StudioEditExecutor::compile(plan, MAX_STUDIO_EDIT_EXECUTION_WINDOWS, cancellation)
            .map_err(map_execution_error)?;
    validate_supported_composition_presence(
        executor.windows(),
        sources.microphone.is_some(),
        sources.system_audio.is_some(),
    )?;
    let approved_profile = profile
        .approved_profile()
        .map(ExportProfileSpec::approved)
        .map(ExportProfileSpec::validate)
        .transpose()
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    if approved_profile.is_some() && sources.microphone.is_none() && sources.system_audio.is_none()
    {
        return Err(NativeExecutionError::InvalidGraph);
    }
    let total_windows = executor.windows().len();
    let mut progress = NativeStudioProgressEmitter::new(|_| {}, total_windows);
    progress.emit(RenderPhase::Preparing, 0, 0)?;
    let canonical = canonicalize_file_sources(&mut sources)?;
    let descriptor = output.as_raw_fd();
    let pipeline = build_edit_pipeline(
        &canonical,
        StudioEditOutput::FileDescriptor(descriptor),
        profile,
    )?;
    let result = execute_windows(
        &pipeline,
        &mut executor,
        &canonical,
        profile,
        cancellation,
        &mut progress,
    );
    if result.is_err() {
        let _ = set_null(&pipeline);
        return result.map(|()| unreachable!());
    }
    set_null(&pipeline)?;
    progress.emit(RenderPhase::Finalizing, 9_800, total_windows)?;
    output
        .sync_all()
        .map_err(|_| NativeExecutionError::Filesystem)?;
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| NativeExecutionError::Filesystem)?;
    let descriptor_path = PathBuf::from(format!("/dev/fd/{descriptor}"));
    let mut artifact = validate_edited_output(
        &descriptor_path,
        profile,
        approved_profile,
        canonical.audio_track_count(),
        executor.windows().len(),
        executor.plan_digest(),
        cancellation,
    )?;
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    artifact.path = artifact_path.to_path_buf();
    progress.emit(RenderPhase::Finalizing, 10_000, total_windows)?;
    Ok(artifact)
}

struct NativeStudioProgressEmitter<F> {
    callback: F,
    total_windows: usize,
    last_phase_rank: u8,
    last_basis_points: u16,
}

impl<F: FnMut(NativeStudioRenderProgress)> NativeStudioProgressEmitter<F> {
    fn new(callback: F, total_windows: usize) -> Self {
        Self {
            callback,
            total_windows,
            last_phase_rank: 0,
            last_basis_points: 0,
        }
    }

    fn emit(
        &mut self,
        phase: RenderPhase,
        basis_points: u16,
        completed_windows: usize,
    ) -> Result<(), NativeExecutionError> {
        let phase_rank = render_phase_rank(phase);
        if self.total_windows == 0
            || completed_windows > self.total_windows
            || basis_points > 10_000
            || basis_points < self.last_basis_points
            || phase_rank < self.last_phase_rank
        {
            return Err(NativeExecutionError::InvalidGraph);
        }
        self.last_phase_rank = phase_rank;
        self.last_basis_points = basis_points;
        (self.callback)(NativeStudioRenderProgress {
            phase,
            basis_points,
            completed_windows,
            total_windows: self.total_windows,
        });
        Ok(())
    }
}

const fn render_phase_rank(phase: RenderPhase) -> u8 {
    match phase {
        RenderPhase::Preparing => 1,
        RenderPhase::Decoding => 2,
        RenderPhase::Compositing => 3,
        RenderPhase::Encoding => 4,
        RenderPhase::Muxing => 5,
        RenderPhase::Finalizing => 6,
    }
}

#[derive(Debug)]
struct StudioOutputReservation {
    final_path: PathBuf,
    staging_directory: PathBuf,
    staging_path: PathBuf,
    identity: Option<Handle>,
    committed: bool,
}

impl StudioOutputReservation {
    fn new(
        final_path: PathBuf,
        profile: NativeStudioExportProfile,
    ) -> Result<Self, NativeExecutionError> {
        if final_path.file_name().is_none() {
            return Err(NativeExecutionError::InvalidOutput);
        }
        reject_existing_output(&final_path)?;
        let parent = output_parent(&final_path);
        create_private_directory(parent)?;
        let mut staging_directory = None;
        for _ in 0..OUTPUT_RESERVATION_ATTEMPTS {
            let candidate = parent.join(format!(".frame-studio-{}", Uuid::new_v4().simple()));
            match fs::create_dir(&candidate) {
                Ok(()) => {
                    create_private_directory(&candidate)?;
                    staging_directory = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(NativeExecutionError::Filesystem),
            }
        }
        let staging_directory = staging_directory.ok_or(NativeExecutionError::Filesystem)?;
        let extension = profile.extension();
        let staging_path = staging_directory.join(format!("artifact.{extension}"));
        Ok(Self {
            final_path,
            staging_directory,
            staging_path,
            identity: None,
            committed: false,
        })
    }

    fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    fn adopt_created(&mut self) -> Result<(), NativeExecutionError> {
        let metadata = fs::symlink_metadata(&self.staging_path)
            .map_err(|_| NativeExecutionError::Filesystem)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(NativeExecutionError::InvalidOutput);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.staging_path)
            .map_err(|_| NativeExecutionError::Filesystem)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| NativeExecutionError::Filesystem)?;
        }
        file.sync_all()
            .map_err(|_| NativeExecutionError::Filesystem)?;
        self.identity = Some(
            Handle::from_file(
                file.try_clone()
                    .map_err(|_| NativeExecutionError::Filesystem)?,
            )
            .map_err(|_| NativeExecutionError::Filesystem)?,
        );
        if !self.path_has_expected_identity(&self.staging_path)? {
            return Err(NativeExecutionError::InvalidOutput);
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<PathBuf, NativeExecutionError> {
        if !self.path_has_expected_identity(&self.staging_path)? {
            return Err(NativeExecutionError::InvalidOutput);
        }
        reject_existing_output(&self.final_path)?;
        fs::hard_link(&self.staging_path, &self.final_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                NativeExecutionError::InvalidOutput
            } else {
                NativeExecutionError::Filesystem
            }
        })?;
        if !self.path_has_expected_identity(&self.final_path)? {
            self.remove_final_if_owned();
            return Err(NativeExecutionError::InvalidOutput);
        }
        if let Err(error) = self.finish_commit() {
            self.remove_final_if_owned();
            return Err(error);
        }
        self.committed = true;
        Ok(self.final_path.clone())
    }

    fn finish_commit(&mut self) -> Result<(), NativeExecutionError> {
        let parent = output_parent(&self.final_path);
        sync_directory(parent)?;
        if !self.path_has_expected_identity(&self.staging_path)? {
            return Err(NativeExecutionError::InvalidOutput);
        }
        fs::remove_file(&self.staging_path).map_err(|_| NativeExecutionError::Filesystem)?;
        fs::remove_dir(&self.staging_directory).map_err(|_| NativeExecutionError::Filesystem)?;
        sync_directory(parent)?;
        if !self.path_has_expected_identity(&self.final_path)? {
            return Err(NativeExecutionError::InvalidOutput);
        }
        Ok(())
    }

    fn path_has_expected_identity(&self, path: &Path) -> Result<bool, NativeExecutionError> {
        let Some(expected) = &self.identity else {
            return Ok(false);
        };
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(NativeExecutionError::Filesystem),
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Ok(false);
        }
        let actual = Handle::from_path(path).map_err(|_| NativeExecutionError::Filesystem)?;
        Ok(actual == *expected)
    }

    fn remove_final_if_owned(&self) {
        if self
            .path_has_expected_identity(&self.final_path)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.final_path);
        }
    }

    fn cleanup_staging(&self) {
        if self.identity.is_none()
            || self
                .path_has_expected_identity(&self.staging_path)
                .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.staging_path);
        }
        let _ = fs::remove_dir(&self.staging_directory);
    }
}

impl Drop for StudioOutputReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.remove_final_if_owned();
            self.cleanup_staging();
        }
    }
}

fn output_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn reject_existing_output(path: &Path) -> Result<(), NativeExecutionError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(NativeExecutionError::InvalidOutput),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(NativeExecutionError::Filesystem),
    }
}

#[derive(Debug)]
struct CanonicalSources {
    screen: PathBuf,
    microphone: Option<PathBuf>,
    system_audio: Option<PathBuf>,
}

#[derive(Debug)]
struct DecoderDispatchBarrier {
    probes: Vec<(gst::Pad, gst::PadProbeId)>,
}

struct TimelinePrerollBarrier {
    probes: Vec<(gst::Pad, gst::PadProbeId)>,
}

impl TimelinePrerollBarrier {
    fn install(
        pipeline: &gst::Pipeline,
        sources: &CanonicalSources,
    ) -> Result<Self, NativeExecutionError> {
        let mut probes = Vec::with_capacity(MAX_STUDIO_ALIGNED_SOURCES);
        for name in sources.timeline_element_names() {
            let pad = pipeline
                .by_name(name)
                .and_then(|element| element.static_pad("src"))
                .ok_or(NativeExecutionError::InvalidGraph)?;
            let probe = pad
                .add_probe(
                    gst::PadProbeType::BLOCK
                        | gst::PadProbeType::BUFFER
                        | gst::PadProbeType::BUFFER_LIST,
                    |_, _| gst::PadProbeReturn::Ok,
                )
                .ok_or(NativeExecutionError::Pipeline)?;
            probes.push((pad, probe));
        }
        Ok(Self { probes })
    }

    fn release(mut self) {
        self.remove_all();
    }

    fn remove_all(&mut self) {
        for (pad, probe) in self.probes.drain(..) {
            pad.remove_probe(probe);
        }
    }
}

impl Drop for TimelinePrerollBarrier {
    fn drop(&mut self) {
        self.remove_all();
    }
}

impl DecoderDispatchBarrier {
    fn install(
        pipeline: &gst::Pipeline,
        sources: &CanonicalSources,
    ) -> Result<Self, NativeExecutionError> {
        let mut probes = Vec::with_capacity(MAX_STUDIO_ALIGNED_SOURCES);
        for name in sources.decode_element_names() {
            let decoder = pipeline
                .by_name(name)
                .ok_or(NativeExecutionError::InvalidGraph)?;
            let mut linked_pads = decoder.src_pads().into_iter().filter(gst::Pad::is_linked);
            let pad = linked_pads
                .next()
                .ok_or(NativeExecutionError::InvalidGraph)?;
            if linked_pads.next().is_some() {
                return Err(NativeExecutionError::InvalidGraph);
            }
            let probe = pad
                .add_probe(
                    gst::PadProbeType::BLOCK
                        | gst::PadProbeType::BUFFER
                        | gst::PadProbeType::BUFFER_LIST,
                    |_, _| gst::PadProbeReturn::Ok,
                )
                .ok_or(NativeExecutionError::Pipeline)?;
            probes.push((pad, probe));
        }
        Ok(Self { probes })
    }

    fn release(mut self) {
        self.remove_all();
    }

    fn remove_all(&mut self) {
        for (pad, probe) in self.probes.drain(..) {
            pad.remove_probe(probe);
        }
    }
}

impl Drop for DecoderDispatchBarrier {
    fn drop(&mut self) {
        self.remove_all();
    }
}

#[derive(Clone)]
struct ObservedVideoFrame {
    buffer: gst::Buffer,
    end_ns: u64,
}

struct ClosingVideoFrameMonitor {
    pad: gst::Pad,
    probe: Option<gst::PadProbeId>,
    observed: Arc<Mutex<Option<ObservedVideoFrame>>>,
    invalid: Arc<AtomicBool>,
}

impl ClosingVideoFrameMonitor {
    fn install(pipeline: &gst::Pipeline) -> Result<Self, NativeExecutionError> {
        let pad = pipeline
            .by_name("edit_screen_timeline")
            .and_then(|timeline| timeline.static_pad("src"))
            .ok_or(NativeExecutionError::InvalidGraph)?;
        let observed = Arc::new(Mutex::new(None::<ObservedVideoFrame>));
        let observed_by_probe = Arc::clone(&observed);
        let invalid = Arc::new(AtomicBool::new(false));
        let invalid_by_probe = Arc::clone(&invalid);
        let probe = pad
            .add_probe(gst::PadProbeType::BUFFER, move |_, information| {
                let Some(gst::PadProbeData::Buffer(buffer)) = information.data.as_ref() else {
                    invalid_by_probe.store(true, AtomicOrdering::Release);
                    return gst::PadProbeReturn::Ok;
                };
                let (Some(pts), Some(duration)) = (buffer.pts(), buffer.duration()) else {
                    invalid_by_probe.store(true, AtomicOrdering::Release);
                    return gst::PadProbeReturn::Ok;
                };
                let Some(end_ns) = pts.nseconds().checked_add(duration.nseconds()) else {
                    invalid_by_probe.store(true, AtomicOrdering::Release);
                    return gst::PadProbeReturn::Ok;
                };
                let Ok(mut current) = observed_by_probe.lock() else {
                    invalid_by_probe.store(true, AtomicOrdering::Release);
                    return gst::PadProbeReturn::Ok;
                };
                if current
                    .as_ref()
                    .is_none_or(|observed| end_ns >= observed.end_ns)
                {
                    *current = Some(ObservedVideoFrame {
                        buffer: buffer.clone(),
                        end_ns,
                    });
                }
                gst::PadProbeReturn::Ok
            })
            .ok_or(NativeExecutionError::Pipeline)?;
        Ok(Self {
            pad,
            probe: Some(probe),
            observed,
            invalid,
        })
    }

    fn close_at(&self, output_end: gst::ClockTime) -> Result<(), NativeExecutionError> {
        if self.invalid.load(AtomicOrdering::Acquire) {
            return Err(NativeExecutionError::InvalidOutput);
        }
        let observed = self
            .observed
            .lock()
            .map_err(|_| NativeExecutionError::Pipeline)?
            .clone()
            .ok_or(NativeExecutionError::InvalidOutput)?;
        let Some(hold_ns) = bounded_video_close_hold_ns(observed.end_ns, output_end.nseconds())?
        else {
            return Ok(());
        };
        let mut duplicate = observed
            .buffer
            .copy_deep()
            .map_err(|_| NativeExecutionError::Pipeline)?;
        let duplicate_ref = duplicate.get_mut().ok_or(NativeExecutionError::Pipeline)?;
        duplicate_ref.set_pts(gst::ClockTime::from_nseconds(observed.end_ns));
        duplicate_ref.set_dts(None);
        duplicate_ref.set_duration(gst::ClockTime::from_nseconds(hold_ns));
        self.pad
            .push(duplicate)
            .map(|_| ())
            .map_err(|_| NativeExecutionError::Pipeline)
    }
}

fn bounded_video_close_hold_ns(
    observed_end_ns: u64,
    output_end_ns: u64,
) -> Result<Option<u64>, NativeExecutionError> {
    if observed_end_ns >= output_end_ns {
        return Ok(None);
    }
    let hold_ns = output_end_ns - observed_end_ns;
    if hold_ns > MAX_STUDIO_VIDEO_CLOSE_HOLD_NS {
        Err(NativeExecutionError::InvalidOutput)
    } else {
        Ok(Some(hold_ns))
    }
}

impl Drop for ClosingVideoFrameMonitor {
    fn drop(&mut self) {
        if let Some(probe) = self.probe.take() {
            self.pad.remove_probe(probe);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BranchSegmentCompletion {
    branch: usize,
    seqnum: gst::Seqnum,
}

struct SegmentCompletionMonitor {
    receiver: Receiver<BranchSegmentCompletion>,
    overflowed: Arc<AtomicBool>,
    probes: Vec<(gst::Pad, gst::PadProbeId)>,
}

impl SegmentCompletionMonitor {
    fn install(
        pipeline: &gst::Pipeline,
        sources: &CanonicalSources,
    ) -> Result<Self, NativeExecutionError> {
        let (sender, receiver) = sync_channel(SEGMENT_COMPLETION_CHANNEL_CAPACITY);
        let overflowed = Arc::new(AtomicBool::new(false));
        let mut probes = Vec::with_capacity(MAX_STUDIO_SEGMENT_BARRIER_BRANCHES);
        for (branch, (name, pad_name)) in sources.segment_completion_probe_specs().enumerate() {
            let pad = pipeline
                .by_name(name)
                .and_then(|element| element.static_pad(pad_name))
                .ok_or(NativeExecutionError::InvalidGraph)?;
            let branch_sender = sender.clone();
            let branch_overflowed = Arc::clone(&overflowed);
            let probe = pad
                .add_probe(
                    gst::PadProbeType::EVENT_DOWNSTREAM,
                    move |_, information| {
                        if let Some(gst::PadProbeData::Event(event)) = information.data.as_ref()
                            && matches!(event.view(), gst::EventView::SegmentDone(_))
                            && branch_sender
                                .try_send(BranchSegmentCompletion {
                                    branch,
                                    seqnum: event.seqnum(),
                                })
                                .is_err()
                        {
                            branch_overflowed.store(true, AtomicOrdering::Release);
                        }
                        gst::PadProbeReturn::Ok
                    },
                )
                .ok_or(NativeExecutionError::Pipeline)?;
            probes.push((pad, probe));
        }
        if probes.is_empty() || probes.len() > MAX_STUDIO_SEGMENT_BARRIER_BRANCHES {
            return Err(NativeExecutionError::InvalidGraph);
        }
        Ok(Self {
            receiver,
            overflowed,
            probes,
        })
    }

    fn branch_count(&self) -> usize {
        self.probes.len()
    }

    fn remove_all(&mut self) {
        for (pad, probe) in self.probes.drain(..) {
            pad.remove_probe(probe);
        }
    }
}

impl Drop for SegmentCompletionMonitor {
    fn drop(&mut self) {
        self.remove_all();
    }
}

#[derive(Debug)]
struct SegmentCompletionState {
    expected_seqnum: gst::Seqnum,
    completed_branches: [bool; MAX_STUDIO_SEGMENT_BARRIER_BRANCHES],
    expected_branches: usize,
}

impl SegmentCompletionState {
    fn new(expected_seqnum: gst::Seqnum, expected_branches: usize) -> Self {
        Self {
            expected_seqnum,
            completed_branches: [false; MAX_STUDIO_SEGMENT_BARRIER_BRANCHES],
            expected_branches,
        }
    }

    fn observe_branch(
        &mut self,
        completion: BranchSegmentCompletion,
    ) -> Result<(), NativeExecutionError> {
        if completion.seqnum != self.expected_seqnum {
            return Ok(());
        }
        let completed = self
            .completed_branches
            .get_mut(completion.branch)
            .filter(|_| completion.branch < self.expected_branches)
            .ok_or(NativeExecutionError::InvalidGraph)?;
        *completed = true;
        Ok(())
    }

    fn complete(&self) -> bool {
        self.completed_branches[..self.expected_branches]
            .iter()
            .all(|completed| *completed)
    }
}

impl CanonicalSources {
    fn audio_track_count(&self) -> u8 {
        u8::from(self.microphone.is_some()) + u8::from(self.system_audio.is_some())
    }

    fn source_element_names(&self) -> impl Iterator<Item = &'static str> {
        [
            Some("edit_screen_source"),
            self.microphone.as_ref().map(|_| "edit_microphone_source"),
            self.system_audio
                .as_ref()
                .map(|_| "edit_system_audio_source"),
        ]
        .into_iter()
        .flatten()
    }

    fn decode_element_names(&self) -> impl Iterator<Item = &'static str> {
        [
            Some("edit_screen_decode"),
            self.microphone.as_ref().map(|_| "edit_microphone_decode"),
            self.system_audio
                .as_ref()
                .map(|_| "edit_system_audio_decode"),
        ]
        .into_iter()
        .flatten()
    }

    fn timeline_element_names(&self) -> impl Iterator<Item = &'static str> {
        [
            Some("edit_screen_timeline"),
            self.microphone.as_ref().map(|_| "edit_microphone_timeline"),
            self.system_audio
                .as_ref()
                .map(|_| "edit_system_audio_timeline"),
        ]
        .into_iter()
        .flatten()
    }

    fn segment_completion_probe_specs(&self) -> impl Iterator<Item = (&'static str, &'static str)> {
        [
            Some(("edit_screen_timeline", "sink")),
            self.microphone
                .as_ref()
                .map(|_| ("edit_microphone_timeline", "sink")),
            self.system_audio
                .as_ref()
                .map(|_| ("edit_system_audio_timeline", "sink")),
        ]
        .into_iter()
        .flatten()
    }
}

fn canonicalize_sources(
    sources: &NativeStudioAlignedSources,
) -> Result<CanonicalSources, NativeExecutionError> {
    Ok(CanonicalSources {
        screen: canonical_regular_file(&sources.screen)?,
        microphone: sources
            .microphone
            .as_ref()
            .map(|path| canonical_regular_file(path))
            .transpose()?,
        system_audio: sources
            .system_audio
            .as_ref()
            .map(|path| canonical_regular_file(path))
            .transpose()?,
    })
}

#[cfg(unix)]
fn canonicalize_file_sources(
    sources: &mut NativeStudioAlignedFileSources,
) -> Result<CanonicalSources, NativeExecutionError> {
    Ok(CanonicalSources {
        screen: canonical_file_descriptor(&mut sources.screen)?,
        microphone: sources
            .microphone
            .as_mut()
            .map(canonical_file_descriptor)
            .transpose()?,
        system_audio: sources
            .system_audio
            .as_mut()
            .map(canonical_file_descriptor)
            .transpose()?,
    })
}

#[cfg(unix)]
fn canonical_file_descriptor(file: &mut File) -> Result<PathBuf, NativeExecutionError> {
    let metadata = file
        .metadata()
        .map_err(|_| NativeExecutionError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(NativeExecutionError::InvalidOutput);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| NativeExecutionError::Filesystem)?;
    Ok(PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd())))
}

fn canonical_regular_file(path: &Path) -> Result<PathBuf, NativeExecutionError> {
    let canonical = fs::canonicalize(path).map_err(|_| NativeExecutionError::Filesystem)?;
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|_| NativeExecutionError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(NativeExecutionError::InvalidOutput);
    }
    Ok(canonical)
}

fn validate_supported_composition(
    windows: &[StudioEditExecutionWindow],
    sources: &NativeStudioAlignedSources,
) -> Result<(), NativeExecutionError> {
    validate_supported_composition_presence(
        windows,
        sources.microphone.is_some(),
        sources.system_audio.is_some(),
    )
}

fn validate_supported_composition_presence(
    windows: &[StudioEditExecutionWindow],
    microphone_supplied: bool,
    system_audio_supplied: bool,
) -> Result<(), NativeExecutionError> {
    let default = crate::CompositeStyle::default();
    for window in windows {
        if window.camera_source_available
            || !matches!(
                window.style.layout,
                LayoutPreset::ScreenOnly | LayoutPreset::CameraBubble
            )
            || window.style.cursor != default.cursor
            || window.style.background != BackgroundStyle::Transparent
        {
            return Err(NativeExecutionError::InvalidGraph);
        }
    }
    validate_audio_source(
        windows
            .iter()
            .any(|window| window.microphone.source_available),
        microphone_supplied,
    )?;
    validate_audio_source(
        windows
            .iter()
            .any(|window| window.system_audio.source_available),
        system_audio_supplied,
    )
}

fn validate_audio_source(required: bool, supplied: bool) -> Result<(), NativeExecutionError> {
    if required == supplied {
        Ok(())
    } else {
        Err(NativeExecutionError::InvalidGraph)
    }
}

#[derive(Debug, Clone, Copy)]
struct NativeStudioEncodingConfig {
    video_transform: &'static str,
    video_encoder: &'static str,
    source_audio_transform: &'static str,
    audio_transform: &'static str,
    audio_encoder: &'static str,
    muxer: &'static str,
    container_marker: &'static [u8],
    video_marker: &'static [u8],
    audio_marker: &'static [u8],
}

const fn studio_encoding_config(profile: NativeStudioExportProfile) -> NativeStudioEncodingConfig {
    match profile {
        NativeStudioExportProfile::EditableWebM => NativeStudioEncodingConfig {
            video_transform: "videoconvert",
            video_encoder: "vp8enc name=edit_video_encoder deadline=1",
            source_audio_transform: "audioconvert ! audioresample",
            audio_transform: "audioconvert ! audioresample",
            audio_encoder: "opusenc",
            muxer: "webmmux streamable=false",
            container_marker: b"webm",
            video_marker: b"V_VP8",
            audio_marker: b"A_OPUS",
        },
        NativeStudioExportProfile::DistributionMasterMp4 => NativeStudioEncodingConfig {
            video_transform: "videoscale add-borders=true ! videorate ! videoconvert gamma-mode=remap primaries-mode=fast ! video/x-raw,format=I420,width=1920,height=1080,framerate=30/1,colorimetry=2:3:5:1",
            video_encoder: "x264enc name=edit_video_encoder bitrate=12000 tune=zerolatency key-int-max=60 byte-stream=false ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au",
            source_audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_encoder: "avenc_aac bitrate=256000 ! aacparse",
            muxer: "mp4mux faststart=true fragment-duration=2000 streamable=true",
            container_marker: b"ftyp",
            video_marker: b"avc1",
            audio_marker: b"mp4a",
        },
        NativeStudioExportProfile::NativeHighQualityWebM => NativeStudioEncodingConfig {
            video_transform: "videoscale add-borders=true ! videorate ! videoconvert gamma-mode=remap primaries-mode=fast ! video/x-raw,format=I420,width=2560,height=1440,framerate=60/1,colorimetry=1:3:5:1",
            video_encoder: "vp8enc name=edit_video_encoder deadline=1 target-bitrate=24000000 keyframe-max-dist=120",
            source_audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_encoder: "opusenc bitrate=320000",
            muxer: "webmmux streamable=false",
            container_marker: b"webm",
            video_marker: b"V_VP8",
            audio_marker: b"A_OPUS",
        },
        NativeStudioExportProfile::NativeHighQualityHevcMp4 => NativeStudioEncodingConfig {
            video_transform: "videoscale add-borders=true ! videorate ! videoconvert gamma-mode=remap primaries-mode=fast ! video/x-raw,format=I420,width=3840,height=2160,framerate=60/1,colorimetry=1:3:7:11",
            video_encoder: "x265enc name=edit_video_encoder bitrate=60000 key-int-max=120 ! h265parse config-interval=-1 ! video/x-h265,stream-format=hvc1,alignment=au",
            source_audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_encoder: "avenc_aac bitrate=320000 ! aacparse",
            muxer: "mp4mux faststart=true fragment-duration=2000 streamable=true",
            container_marker: b"ftyp",
            video_marker: b"hvc1",
            audio_marker: b"mp4a",
        },
        NativeStudioExportProfile::NativeArchiveMatroska => NativeStudioEncodingConfig {
            video_transform: "videoscale add-borders=true ! videorate ! videoconvert gamma-mode=remap primaries-mode=fast ! video/x-raw,format=I420,width=3840,height=2160,framerate=60/1,colorimetry=1:3:5:1",
            video_encoder: "avenc_ffv1 name=edit_video_encoder",
            source_audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_transform: "audioconvert ! audioresample ! audio/x-raw,format=S24_32LE,layout=interleaved,rate=48000,channels=2,channel-mask=(bitmask)0x3",
            audio_encoder: "flacenc",
            muxer: "matroskamux streamable=false",
            container_marker: b"matroska",
            video_marker: b"V_FFV1",
            audio_marker: b"A_FLAC",
        },
    }
}

enum StudioEditOutput<'a> {
    Path(&'a Path),
    #[cfg(unix)]
    FileDescriptor(std::os::fd::RawFd),
}

fn build_edit_pipeline(
    sources: &CanonicalSources,
    output: StudioEditOutput<'_>,
    profile: NativeStudioExportProfile,
) -> Result<gst::Pipeline, NativeExecutionError> {
    prepare_runtime().map_err(|_| NativeExecutionError::MissingFactory)?;
    let config = studio_encoding_config(profile);
    let output_sink = match output {
        StudioEditOutput::Path(_) => "filesink name=edit_output sync=false async=false",
        #[cfg(unix)]
        StudioEditOutput::FileDescriptor(_) => "fdsink name=edit_output sync=false async=false",
    };
    let mut description = format!(
        concat!(
            "{muxer} name=edit_mux ! {output_sink} ",
            "filesrc name=edit_screen_source ! decodebin name=edit_screen_decode ",
            "edit_screen_decode. ! queue name=edit_screen_input_queue max-size-buffers=64 max-size-bytes=134217728 max-size-time=2000000000 ",
            "! videoconvert ! identity name=edit_screen_timeline single-segment=true ",
            "! {video_transform} ! {video_encoder} ",
            "! queue name=edit_video_output_queue max-size-buffers=64 max-size-bytes=67108864 max-size-time=2000000000 ! edit_mux. "
        ),
        muxer = config.muxer,
        output_sink = output_sink,
        video_transform = config.video_transform,
        video_encoder = config.video_encoder,
    );
    if sources.audio_track_count() > 0 {
        description.push_str(&format!(
            concat!(
                "audiomixer name=edit_audio_mixer ignore-inactive-pads=true ",
                "! queue name=edit_audio_mix_queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ",
                "! {audio_transform} ! {audio_encoder} ",
                "! queue name=edit_audio_output_queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ! edit_mux. "
            ),
            audio_transform = config.audio_transform,
            audio_encoder = config.audio_encoder,
        ));
    }
    if sources.microphone.is_some() {
        description.push_str(&format!(
            concat!(
                "filesrc name=edit_microphone_source ! decodebin name=edit_microphone_decode ",
                "edit_microphone_decode. ! queue name=edit_microphone_input_queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ",
                "! {source_audio_transform} ! identity name=edit_microphone_timeline single-segment=true ",
                "! volume name=edit_microphone_volume ! edit_audio_mixer. "
            ),
            source_audio_transform = config.source_audio_transform,
        ));
    }
    if sources.system_audio.is_some() {
        description.push_str(&format!(
            concat!(
                "filesrc name=edit_system_audio_source ! decodebin name=edit_system_audio_decode ",
                "edit_system_audio_decode. ! queue name=edit_system_audio_input_queue max-size-buffers=128 max-size-bytes=8388608 max-size-time=2000000000 ",
                "! {source_audio_transform} ! identity name=edit_system_audio_timeline single-segment=true ",
                "! volume name=edit_system_audio_volume ! edit_audio_mixer. "
            ),
            source_audio_transform = config.source_audio_transform,
        ));
    }
    if description.len() > 64 * 1024 {
        return Err(NativeExecutionError::InvalidGraph);
    }
    let pipeline = gst::parse::launch(&description)
        .map_err(|_| NativeExecutionError::MissingFactory)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    pipeline
        .by_name("edit_screen_source")
        .ok_or(NativeExecutionError::InvalidGraph)?
        .set_property("location", &sources.screen);
    if let Some(path) = &sources.microphone {
        pipeline
            .by_name("edit_microphone_source")
            .ok_or(NativeExecutionError::InvalidGraph)?
            .set_property("location", path);
    }
    if let Some(path) = &sources.system_audio {
        pipeline
            .by_name("edit_system_audio_source")
            .ok_or(NativeExecutionError::InvalidGraph)?
            .set_property("location", path);
    }
    let output_element = pipeline
        .by_name("edit_output")
        .ok_or(NativeExecutionError::InvalidGraph)?;
    match output {
        StudioEditOutput::Path(path) => output_element.set_property("location", path),
        #[cfg(unix)]
        StudioEditOutput::FileDescriptor(descriptor) => {
            output_element.set_property("fd", descriptor);
        }
    }
    require_trusted(&pipeline)?;
    Ok(pipeline)
}

fn require_trusted(pipeline: &gst::Pipeline) -> Result<(), NativeExecutionError> {
    if !pipeline_has_trusted_factory_provenance(pipeline)
        || !pipeline_has_only_declared_authored_factories(pipeline)
    {
        return Err(NativeExecutionError::UntrustedFactory);
    }
    let names = pipeline
        .children()
        .iter()
        .filter_map(|element| element.factory().map(|factory| factory.name().to_string()))
        .collect::<BTreeSet<_>>();
    if names.is_empty() {
        Err(NativeExecutionError::InvalidGraph)
    } else {
        Ok(())
    }
}

fn execute_windows<F: FnMut(NativeStudioRenderProgress)>(
    pipeline: &gst::Pipeline,
    executor: &mut StudioEditExecutor,
    sources: &CanonicalSources,
    profile: NativeStudioExportProfile,
    cancellation: &CancellationToken,
    progress: &mut NativeStudioProgressEmitter<F>,
) -> Result<(), NativeExecutionError> {
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    // Non-live file sources can otherwise run through EOS while an async=false
    // output sink lets the graph settle in Paused. Hold one buffer per aligned
    // timeline before preroll so the first canonical FLUSH seek discards all
    // speculative bytes before any encoder can finalize.
    let mut initial_preroll = Some(TimelinePrerollBarrier::install(pipeline, sources)?);
    if pipeline.set_state(gst::State::Paused).is_err() {
        return Err(NativeExecutionError::Pipeline);
    }
    let (transition, current, _) = pipeline.state(EDIT_STATE_TIMEOUT);
    if transition.is_err() || current != gst::State::Paused {
        return Err(NativeExecutionError::Pipeline);
    }
    // decodebin descendants do not exist at authored-graph validation time.
    // Once preroll has autoplugged them, every decoder/demuxer must still come
    // from the build-time trusted plugin root before any edited bytes execute.
    if !pipeline_has_trusted_factory_provenance(pipeline) {
        return Err(NativeExecutionError::UntrustedFactory);
    }
    let flushed_encoder_caps = flushed_encoder_caps(profile)?;
    progress.emit(RenderPhase::Decoding, 1_000, 0)?;
    let bus = pipeline.bus().ok_or(NativeExecutionError::Pipeline)?;
    let segment_monitor = SegmentCompletionMonitor::install(pipeline, sources)?;
    let closing_video_frame = ClosingVideoFrameMonitor::install(pipeline)?;
    progress.emit(RenderPhase::Compositing, 2_000, 0)?;
    let output_end = exact_duration_to_clock_time_ceil(executor.output_duration())?;
    let deadline = Instant::now() + EDIT_EXECUTION_DEADLINE;
    let mut messages = 0_usize;
    let mut first = true;
    let mut completed_windows = 0_usize;
    while executor.remaining_windows() > 0 {
        let batch = executor
            .next_batch(MAX_STUDIO_EDIT_EXECUTION_BATCH, cancellation)
            .map_err(map_execution_error)?
            .to_vec();
        for window in batch {
            if cancellation.is_cancelled() {
                return Err(NativeExecutionError::Cancelled);
            }
            if !first {
                if pipeline.set_state(gst::State::Paused).is_err() {
                    return Err(NativeExecutionError::Pipeline);
                }
                let (transition, current, _) = pipeline.state(EDIT_STATE_TIMEOUT);
                if transition.is_err() || current != gst::State::Paused {
                    return Err(NativeExecutionError::Pipeline);
                }
            }
            apply_audio_window(pipeline, window, sources)?;
            let mut flags = gst::SeekFlags::ACCURATE | gst::SeekFlags::SEGMENT;
            if first {
                flags |= gst::SeekFlags::FLUSH;
            }
            let rate = f64::from(window.speed_numerator) / f64::from(window.speed_denominator);
            let start = rational_to_clock_time(window.source_start, false)?;
            let stop = rational_to_clock_time(window.source_end, true)?;
            // Keep every aligned decoder paused and block its first decoded
            // buffer while dispatching copies of one logical seek. Even when
            // a whole short segment fits in downstream preroll, no fast branch
            // can complete before all branches own the shared sequence number.
            let dispatch_barrier = DecoderDispatchBarrier::install(pipeline, sources)?;
            let segment_seqnum = gst::Seqnum::next();
            if first {
                reassert_flushed_encoder_caps(pipeline, flushed_encoder_caps.as_ref())?;
            }
            for name in sources.decode_element_names() {
                let decoder = pipeline
                    .by_name(name)
                    .ok_or(NativeExecutionError::InvalidGraph)?;
                let seek = gst::event::Seek::builder(
                    rate,
                    flags,
                    gst::SeekType::Set,
                    start,
                    gst::SeekType::Set,
                    stop,
                )
                .seqnum(segment_seqnum)
                .build();
                if !decoder.send_event(seek) {
                    return Err(NativeExecutionError::Pipeline);
                }
            }
            dispatch_barrier.release();
            if let Some(barrier) = initial_preroll.take() {
                barrier.release();
            }
            if pipeline.set_state(gst::State::Playing).is_err() {
                return Err(NativeExecutionError::Pipeline);
            }
            if first {
                first = false;
            }
            wait_for_segment_done(
                &bus,
                &segment_monitor,
                segment_seqnum,
                cancellation,
                deadline,
                &mut messages,
            )?;
            completed_windows = completed_windows
                .checked_add(1)
                .ok_or(NativeExecutionError::ResourceLimit)?;
            let completed = u64::try_from(completed_windows)
                .map_err(|_| NativeExecutionError::ResourceLimit)?;
            let total = u64::try_from(progress.total_windows)
                .map_err(|_| NativeExecutionError::ResourceLimit)?;
            let basis_points = 2_000_u64
                .checked_add(
                    7_000_u64
                        .checked_mul(completed)
                        .ok_or(NativeExecutionError::ResourceLimit)?
                        / total,
                )
                .ok_or(NativeExecutionError::ResourceLimit)?;
            progress.emit(
                RenderPhase::Encoding,
                u16::try_from(basis_points).map_err(|_| NativeExecutionError::ResourceLimit)?,
                completed_windows,
            )?;
        }
    }
    closing_video_frame.close_at(output_end)?;
    for name in sources.source_element_names() {
        let pad = pipeline
            .by_name(name)
            .and_then(|source| source.static_pad("src"))
            .ok_or(NativeExecutionError::InvalidGraph)?;
        if !pad.push_event(gst::event::Eos::new()) {
            return Err(NativeExecutionError::Pipeline);
        }
    }
    wait_for_eos(&bus, cancellation, deadline, &mut messages)?;
    progress.emit(RenderPhase::Muxing, 9_500, completed_windows)
}

fn flushed_encoder_caps(
    profile: NativeStudioExportProfile,
) -> Result<Option<gst::Caps>, NativeExecutionError> {
    let Some(description) = (profile == NativeStudioExportProfile::NativeHighQualityHevcMp4)
        .then_some(
            "video/x-raw,format=I420,width=3840,height=2160,framerate=60/1,colorimetry=1:3:7:11",
        )
    else {
        return Ok(None);
    };
    let caps = description
        .parse::<gst::Caps>()
        .map_err(|_| NativeExecutionError::InvalidGraph)?;
    if caps.is_fixed() {
        Ok(Some(caps))
    } else {
        Err(NativeExecutionError::InvalidGraph)
    }
}

fn reassert_flushed_encoder_caps(
    pipeline: &gst::Pipeline,
    caps: Option<&gst::Caps>,
) -> Result<(), NativeExecutionError> {
    let Some(caps) = caps else {
        return Ok(());
    };
    let sink = pipeline
        .by_name("edit_video_encoder")
        .and_then(|encoder| encoder.static_pad("sink"))
        .ok_or(NativeExecutionError::InvalidGraph)?;
    if sink.send_event(gst::event::Caps::new(caps)) {
        Ok(())
    } else {
        Err(NativeExecutionError::Pipeline)
    }
}

fn apply_audio_window(
    pipeline: &gst::Pipeline,
    window: StudioEditExecutionWindow,
    sources: &CanonicalSources,
) -> Result<(), NativeExecutionError> {
    if sources.microphone.is_some() {
        set_volume(
            pipeline,
            "edit_microphone_volume",
            window.microphone.source_available,
            window.microphone.style.gain_millibels,
            window.microphone.style.muted,
        )?;
    }
    if sources.system_audio.is_some() {
        set_volume(
            pipeline,
            "edit_system_audio_volume",
            window.system_audio.source_available,
            window.system_audio.style.gain_millibels,
            window.system_audio.style.muted,
        )?;
    }
    Ok(())
}

fn set_volume(
    pipeline: &gst::Pipeline,
    name: &str,
    source_available: bool,
    gain_millibels: i32,
    muted: bool,
) -> Result<(), NativeExecutionError> {
    let volume = if source_available && !muted {
        10_f64.powf(f64::from(gain_millibels) / 2_000.0)
    } else {
        0.0
    };
    pipeline
        .by_name(name)
        .ok_or(NativeExecutionError::InvalidGraph)?
        .set_property("volume", volume);
    Ok(())
}

fn wait_for_segment_done(
    bus: &gst::Bus,
    monitor: &SegmentCompletionMonitor,
    expected_seqnum: gst::Seqnum,
    cancellation: &CancellationToken,
    deadline: Instant,
    messages: &mut usize,
) -> Result<(), NativeExecutionError> {
    let mut state = SegmentCompletionState::new(expected_seqnum, monitor.branch_count());
    loop {
        if monitor.overflowed.load(AtomicOrdering::Acquire) {
            return Err(NativeExecutionError::ResourceLimit);
        }
        while let Ok(completion) = monitor.receiver.try_recv() {
            state.observe_branch(completion)?;
        }
        if state.complete() {
            return Ok(());
        }
        let Some(message) = poll_message(bus, cancellation, deadline, messages)? else {
            continue;
        };
        match message.view() {
            gst::MessageView::Error(_) => return Err(NativeExecutionError::Pipeline),
            gst::MessageView::Eos(_) => return Err(NativeExecutionError::InvalidOutput),
            _ => {}
        }
    }
}

fn wait_for_eos(
    bus: &gst::Bus,
    cancellation: &CancellationToken,
    deadline: Instant,
    messages: &mut usize,
) -> Result<(), NativeExecutionError> {
    loop {
        let message = next_message(bus, cancellation, deadline, messages)?;
        match message.view() {
            gst::MessageView::Eos(_) => return Ok(()),
            gst::MessageView::Error(_) => return Err(NativeExecutionError::Pipeline),
            _ => {}
        }
    }
}

fn next_message(
    bus: &gst::Bus,
    cancellation: &CancellationToken,
    deadline: Instant,
    messages: &mut usize,
) -> Result<gst::Message, NativeExecutionError> {
    loop {
        if let Some(message) = poll_message(bus, cancellation, deadline, messages)? {
            return Ok(message);
        }
    }
}

fn poll_message(
    bus: &gst::Bus,
    cancellation: &CancellationToken,
    deadline: Instant,
    messages: &mut usize,
) -> Result<Option<gst::Message>, NativeExecutionError> {
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(NativeExecutionError::Timeout);
    }
    let Some(message) = bus.timed_pop(EDIT_BUS_POLL) else {
        return Ok(None);
    };
    *messages = messages
        .checked_add(1)
        .ok_or(NativeExecutionError::ResourceLimit)?;
    if *messages > MAX_EDIT_BUS_MESSAGES {
        return Err(NativeExecutionError::ResourceLimit);
    }
    Ok(Some(message))
}

fn rational_to_clock_time(
    value: crate::RationalTime,
    ceil: bool,
) -> Result<gst::ClockTime, NativeExecutionError> {
    let numerator = u128::from(value.ticks())
        .checked_mul(1_000_000_000)
        .ok_or(NativeExecutionError::ResourceLimit)?;
    let denominator = u128::from(value.time_base().ticks_per_second());
    let nanos = if ceil {
        numerator
            .checked_add(denominator - 1)
            .ok_or(NativeExecutionError::ResourceLimit)?
            / denominator
    } else {
        numerator / denominator
    };
    Ok(gst::ClockTime::from_nseconds(
        u64::try_from(nanos).map_err(|_| NativeExecutionError::ResourceLimit)?,
    ))
}

fn exact_duration_to_std(value: ExactDuration) -> Result<Duration, NativeExecutionError> {
    let nanos = value
        .numerator()
        .checked_mul(1_000_000_000)
        .ok_or(NativeExecutionError::ResourceLimit)?
        / value.denominator();
    Ok(Duration::from_nanos(
        u64::try_from(nanos).map_err(|_| NativeExecutionError::ResourceLimit)?,
    ))
}

fn exact_duration_to_clock_time_ceil(
    value: ExactDuration,
) -> Result<gst::ClockTime, NativeExecutionError> {
    let numerator = value
        .numerator()
        .checked_mul(1_000_000_000)
        .ok_or(NativeExecutionError::ResourceLimit)?;
    let nanos = numerator
        .checked_add(value.denominator() - 1)
        .ok_or(NativeExecutionError::ResourceLimit)?
        / value.denominator();
    Ok(gst::ClockTime::from_nseconds(
        u64::try_from(nanos).map_err(|_| NativeExecutionError::ResourceLimit)?,
    ))
}

fn validate_edited_output(
    output: &Path,
    profile: NativeStudioExportProfile,
    approved_profile: Option<ExportProfileSpec>,
    audio_tracks: u8,
    execution_windows: usize,
    plan_digest: Sha256Digest,
    cancellation: &CancellationToken,
) -> Result<NativeStudioEditedExportArtifact, NativeExecutionError> {
    let metadata = fs::symlink_metadata(output).map_err(|_| NativeExecutionError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() < 128
    {
        return Err(NativeExecutionError::InvalidOutput);
    }
    let probe_len = usize::try_from(metadata.len().min(MAX_EDIT_OUTPUT_PROBE_BYTES))
        .map_err(|_| NativeExecutionError::ResourceLimit)?;
    let mut prefix = vec![0_u8; probe_len];
    File::open(output)
        .and_then(|mut file| file.read_exact(&mut prefix))
        .map_err(|_| NativeExecutionError::Filesystem)?;
    let config = studio_encoding_config(profile);
    let container_marker = config.container_marker;
    let playable_container_marker = prefix
        .windows(container_marker.len())
        .any(|value| value == container_marker);
    let video_marker = prefix
        .windows(config.video_marker.len())
        .any(|value| value == config.video_marker);
    let audio_marker = prefix
        .windows(config.audio_marker.len())
        .any(|value| value == config.audio_marker);
    if !playable_container_marker || !video_marker || (audio_tracks > 0 && !audio_marker) {
        return Err(NativeExecutionError::InvalidOutput);
    }
    let (video, audio) = probe_edited_output(output, audio_tracks, cancellation)?;
    if let Some(expected) = approved_profile {
        validate_approved_output_caps(&video, audio, expected, audio_tracks)?;
    }
    Ok(NativeStudioEditedExportArtifact {
        profile,
        approved_profile,
        path: output.to_path_buf(),
        bytes: metadata.len(),
        sha256: sha256_file_with_budget(output, cancellation, EDIT_EXECUTION_DEADLINE)?,
        playable_container_marker,
        audio_tracks,
        video,
        audio,
        execution_windows,
        plan_digest,
    })
}

fn probe_edited_output(
    output: &Path,
    audio_tracks: u8,
    cancellation: &CancellationToken,
) -> Result<(NativeStudioOutputVideo, Option<NativeStudioOutputAudio>), NativeExecutionError> {
    if cancellation.is_cancelled() {
        return Err(NativeExecutionError::Cancelled);
    }
    let player = software_output_probe_player()?;
    let video_sink = gst::ElementFactory::make("fakesink")
        .name("studio_export_probe_video")
        .property("sync", false)
        .build()
        .map_err(|_| NativeExecutionError::MissingFactory)?;
    let audio_sink = gst::ElementFactory::make("fakesink")
        .name("studio_export_probe_audio")
        .property("sync", false)
        .build()
        .map_err(|_| NativeExecutionError::MissingFactory)?;
    player.set_property("video-sink", &video_sink);
    player.set_property("audio-sink", &audio_sink);
    let uri =
        gst::glib::filename_to_uri(output, None).map_err(|_| NativeExecutionError::Filesystem)?;
    player.set_property("uri", uri.as_str());
    let pipeline = gst::Pipeline::with_name("studio_export_probe_pipeline");
    pipeline
        .add(&player)
        .map_err(|_| NativeExecutionError::Pipeline)?;
    require_trusted(&pipeline)?;
    let result = (|| {
        if pipeline.set_state(gst::State::Paused).is_err() {
            return Err(NativeExecutionError::Pipeline);
        }
        let (transition, current, _) = pipeline.state(EDIT_STATE_TIMEOUT);
        if transition.is_err() || current != gst::State::Paused {
            return Err(NativeExecutionError::InvalidOutput);
        }
        if cancellation.is_cancelled() {
            return Err(NativeExecutionError::Cancelled);
        }
        if !pipeline_has_trusted_factory_provenance(&pipeline) {
            return Err(NativeExecutionError::UntrustedFactory);
        }
        let video_caps = video_sink
            .static_pad("sink")
            .and_then(|pad| pad.current_caps())
            .ok_or(NativeExecutionError::InvalidOutput)?;
        let video = output_video_from_caps(&video_caps)?;
        let audio_caps = audio_sink
            .static_pad("sink")
            .and_then(|pad| pad.current_caps());
        let audio = audio_caps
            .as_ref()
            .map(|caps| output_audio_from_caps(caps.as_ref()))
            .transpose()?;
        if (audio_tracks == 0) != audio.is_none() {
            return Err(NativeExecutionError::InvalidOutput);
        }
        Ok((video, audio))
    })();
    set_null(&pipeline)?;
    result
}

fn software_output_probe_player() -> Result<gst::Element, NativeExecutionError> {
    // playbin3 documents that its software-decoder flag has no effect. Use
    // playbin2 for the postcondition probe so a failing platform decoder cannot
    // mask a valid software-encoded artifact.
    let player = gst::ElementFactory::make("playbin")
        .name("studio_export_probe_player")
        .build()
        .map_err(|_| NativeExecutionError::MissingFactory)?;
    let flags_spec = player
        .find_property("flags")
        .ok_or(NativeExecutionError::MissingFactory)?;
    let flags = gst::glib::Value::deserialize_with_pspec(
        "video+audio+soft-volume+force-sw-decoders",
        &flags_spec,
    )
    .map_err(|_| NativeExecutionError::MissingFactory)?;
    player.set_property_from_value("flags", &flags);
    Ok(player)
}

fn output_video_from_caps(
    caps: &gst::CapsRef,
) -> Result<NativeStudioOutputVideo, NativeExecutionError> {
    let structure = caps
        .structure(0)
        .ok_or(NativeExecutionError::InvalidOutput)?;
    let width = structure
        .get::<i32>("width")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(NativeExecutionError::InvalidOutput)?;
    let height = structure
        .get::<i32>("height")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(NativeExecutionError::InvalidOutput)?;
    let fraction = structure
        .get::<gst::Fraction>("framerate")
        .map_err(|_| NativeExecutionError::InvalidOutput)?;
    let numerator =
        u32::try_from(fraction.numer()).map_err(|_| NativeExecutionError::InvalidOutput)?;
    let denominator =
        u32::try_from(fraction.denom()).map_err(|_| NativeExecutionError::InvalidOutput)?;
    let frame_rate = FrameRate {
        numerator,
        denominator,
    };
    frame_rate
        .validate()
        .map_err(|_| NativeExecutionError::InvalidOutput)?;
    let colorimetry = structure
        .get::<String>("colorimetry")
        .unwrap_or_else(|_| "unknown".to_owned());
    let colorimetry = match colorimetry.as_str() {
        // GStreamer defines `bt709` as the canonical alias for limited-range
        // BT.709 (range/matrix/transfer/primaries = 2:3:5:1).
        "bt709" => "2:3:5:1".to_owned(),
        _ => colorimetry,
    };
    if colorimetry.len() > 64 {
        return Err(NativeExecutionError::InvalidOutput);
    }
    Ok(NativeStudioOutputVideo {
        width,
        height,
        frame_rate,
        colorimetry,
    })
}

fn output_audio_from_caps(
    caps: &gst::CapsRef,
) -> Result<NativeStudioOutputAudio, NativeExecutionError> {
    let structure = caps
        .structure(0)
        .ok_or(NativeExecutionError::InvalidOutput)?;
    let sample_rate = structure
        .get::<i32>("rate")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(NativeExecutionError::InvalidOutput)?;
    let channels = structure
        .get::<i32>("channels")
        .ok()
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(NativeExecutionError::InvalidOutput)?;
    Ok(NativeStudioOutputAudio {
        sample_rate,
        channels,
    })
}

fn validate_approved_output_caps(
    video: &NativeStudioOutputVideo,
    audio: Option<NativeStudioOutputAudio>,
    expected: ExportProfileSpec,
    audio_tracks: u8,
) -> Result<(), NativeExecutionError> {
    if audio_tracks == 0 {
        return Err(NativeExecutionError::InvalidOutput);
    }
    let expected_colorimetry = match expected.color {
        StudioColorSpace::Bt709Limited => "2:3:5:1",
        StudioColorSpace::Bt709Full => "1:3:5:1",
        StudioColorSpace::DisplayP3 => "1:3:7:11",
    };
    if video.width != expected.resolution.width
        || video.height != expected.resolution.height
        || video.frame_rate != expected.frame_rate
        || video.colorimetry != expected_colorimetry
    {
        return Err(NativeExecutionError::InvalidOutput);
    }
    match audio {
        Some(audio)
            if audio.sample_rate == expected.audio_sample_rate
                && audio.channels == expected.audio_channels =>
        {
            Ok(())
        }
        None | Some(_) => Err(NativeExecutionError::InvalidOutput),
    }
}

fn map_execution_error(error: StudioEditExecutionError) -> NativeExecutionError {
    match error {
        StudioEditExecutionError::Cancelled => NativeExecutionError::Cancelled,
        StudioEditExecutionError::InvalidWindowLimit
        | StudioEditExecutionError::WindowLimitExceeded
        | StudioEditExecutionError::InvalidBatchLimit => NativeExecutionError::ResourceLimit,
        StudioEditExecutionError::InvalidPlan(_)
        | StudioEditExecutionError::OutputOutsideTimeline => NativeExecutionError::InvalidGraph,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AssetChecksum, AssetCommitState, AudioSampleFormat, EditOperation, EditSpec,
        NativeStudioTrackRole, PixelFormat, SourceCoverage, StudioAssetCodec, StudioAssetEncoding,
        StudioAssetRawCaps, StudioAudioCodec, StudioAudioRawCaps, StudioProjectId,
        StudioProjectManifest, StudioSourceName, StudioSourceSet, StudioState,
        StudioTimelineCompiler, StudioVideoCodec, StudioVideoRawCaps, TempAssetCommitTicket,
        TimeBase, TimelineSource, TrackKind, commit_verified_temporary,
        record_synthetic_studio_tracks,
    };
    use std::collections::BTreeMap;

    fn media_duration(path: &Path) -> gst::ClockTime {
        prepare_runtime().expect("trusted test runtime");
        let player = gst::ElementFactory::make("playbin3")
            .build()
            .expect("playbin3");
        let video_sink = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
            .expect("video fakesink");
        let audio_sink = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
            .expect("audio fakesink");
        player.set_property("video-sink", &video_sink);
        player.set_property("audio-sink", &audio_sink);
        player.set_property(
            "uri",
            gst::glib::filename_to_uri(path, None)
                .expect("file URI")
                .as_str(),
        );
        let pipeline = gst::Pipeline::new();
        pipeline.add(&player).expect("add player");
        pipeline
            .set_state(gst::State::Paused)
            .expect("pause player");
        let (transition, current, _) = pipeline.state(gst::ClockTime::from_seconds(10));
        assert!(transition.is_ok());
        assert_eq!(current, gst::State::Paused);
        let duration = pipeline
            .query_duration::<gst::ClockTime>()
            .expect("media duration");
        pipeline.set_state(gst::State::Null).expect("Null player");
        duration
    }

    fn quarter(ticks: u64) -> crate::RationalTime {
        crate::RationalTime::new(ticks, TimeBase::new(4).expect("quarter-second timebase"))
    }

    fn edited_plan() -> CanonicalEditPlan {
        let source = TimelineSource {
            duration: quarter(8),
            coverage: vec![
                SourceCoverage {
                    track: TrackKind::Screen,
                    start: quarter(0),
                    end: quarter(8),
                },
                SourceCoverage {
                    track: TrackKind::SystemAudio,
                    start: quarter(0),
                    end: quarter(8),
                },
            ],
            vfr_video_pts: BTreeMap::new(),
        };
        StudioTimelineCompiler::compile(
            &source,
            &EditSpec {
                version: crate::STUDIO_EDIT_VERSION,
                revision: 2,
                operations: vec![
                    EditOperation::Trim {
                        start: quarter(1),
                        end: quarter(7),
                    },
                    EditOperation::DeleteRange {
                        start: quarter(2),
                        end: quarter(3),
                    },
                    EditOperation::Speed {
                        start: quarter(4),
                        end: quarter(6),
                        numerator: 2,
                        denominator: 1,
                    },
                    EditOperation::AudioGain {
                        track: TrackKind::SystemAudio,
                        start: quarter(4),
                        end: quarter(6),
                        gain_millibels: -600,
                        muted: false,
                    },
                ],
            },
        )
        .expect("edited plan")
    }

    fn short_profile_plan(with_audio: bool) -> CanonicalEditPlan {
        profile_plan(15, with_audio)
    }

    fn profile_plan(ticks: u64, with_audio: bool) -> CanonicalEditPlan {
        let time_base = TimeBase::new(30).expect("30 Hz timebase");
        let end = crate::RationalTime::new(ticks, time_base);
        let mut coverage = vec![SourceCoverage {
            track: TrackKind::Screen,
            start: crate::RationalTime::new(0, time_base),
            end,
        }];
        if with_audio {
            coverage.push(SourceCoverage {
                track: TrackKind::SystemAudio,
                start: crate::RationalTime::new(0, time_base),
                end,
            });
        }
        StudioTimelineCompiler::compile(
            &TimelineSource {
                duration: end,
                coverage,
                vfr_video_pts: BTreeMap::new(),
            },
            &EditSpec {
                version: crate::STUDIO_EDIT_VERSION,
                revision: 1,
                operations: Vec::new(),
            },
        )
        .expect("short profile plan")
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_preview_video_asset(
        store: &mut FilesystemStudioOriginalStore,
        project_id: StudioProjectId,
        path: &Path,
        id_marker: u8,
        operation_marker: u8,
        track: TrackKind,
        source_name: &str,
        start: crate::RationalTime,
        duration: crate::RationalTime,
    ) -> StudioAsset {
        let bytes = fs::read(path).expect("synthetic track bytes");
        let asset = StudioAsset {
            version: crate::STUDIO_ASSET_VERSION,
            id: StudioAssetId::from_csprng([id_marker; 16]).expect("asset ID"),
            track,
            source_name: StudioSourceName::new(source_name).expect("portable source name"),
            byte_len: u64::try_from(bytes.len()).expect("asset length"),
            start,
            duration,
            checksum: AssetChecksum::from_content(&bytes),
            commit_state: AssetCommitState::Temporary,
            encoding: StudioAssetEncoding::Encoded {
                container: crate::MediaContainer::WebM,
                codec: StudioAssetCodec::Video(StudioVideoCodec::Vp8),
                raw_caps: StudioAssetRawCaps::Video(StudioVideoRawCaps {
                    width: 320,
                    height: 180,
                    frame_rate: FrameRate {
                        numerator: 30,
                        denominator: 1,
                    },
                    pixel_format: PixelFormat::Bgra8,
                }),
                time_base: TimeBase::new(90_000).expect("video timebase"),
            },
        };
        store
            .stage_temporary_bytes(project_id, &asset, &bytes)
            .expect("stage preview original");
        commit_verified_temporary(
            store,
            TempAssetCommitTicket::new(
                project_id,
                crate::StudioOperationId::from_csprng([operation_marker; 16])
                    .expect("operation ID"),
                1,
                asset,
            )
            .expect("asset commit ticket"),
        )
        .expect("commit preview original")
    }

    fn commit_preview_audio_asset(
        store: &mut FilesystemStudioOriginalStore,
        project_id: StudioProjectId,
        path: &Path,
        id_marker: u8,
        operation_marker: u8,
    ) -> StudioAsset {
        let bytes = fs::read(path).expect("synthetic audio bytes");
        let asset = StudioAsset {
            version: crate::STUDIO_ASSET_VERSION,
            id: StudioAssetId::from_csprng([id_marker; 16]).expect("asset ID"),
            track: TrackKind::SystemAudio,
            source_name: StudioSourceName::new("system-audio-000.webm")
                .expect("portable source name"),
            byte_len: u64::try_from(bytes.len()).expect("asset length"),
            start: quarter(0),
            duration: quarter(8),
            checksum: AssetChecksum::from_content(&bytes),
            commit_state: AssetCommitState::Temporary,
            encoding: StudioAssetEncoding::Encoded {
                container: crate::MediaContainer::WebM,
                codec: StudioAssetCodec::Audio(StudioAudioCodec::Opus),
                raw_caps: StudioAssetRawCaps::Audio(StudioAudioRawCaps {
                    sample_rate: 48_000,
                    channels: 2,
                    sample_format: AudioSampleFormat::Float32,
                }),
                time_base: TimeBase::new(48_000).expect("audio timebase"),
            },
        };
        store
            .stage_temporary_bytes(project_id, &asset, &bytes)
            .expect("stage preview audio");
        commit_verified_temporary(
            store,
            TempAssetCommitTicket::new(
                project_id,
                crate::StudioOperationId::from_csprng([operation_marker; 16])
                    .expect("operation ID"),
                1,
                asset,
            )
            .expect("audio commit ticket"),
        )
        .expect("commit preview audio")
    }

    fn preview_graph_fixture(
        directory: &Path,
    ) -> (
        StudioPreviewGraphSpec,
        FilesystemStudioOriginalStore,
        PathBuf,
        StudioAssetId,
    ) {
        let tracks = record_synthetic_studio_tracks(
            &directory.join("encoded"),
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .expect("synthetic preview originals");
        let source_path = |role| {
            tracks
                .iter()
                .find(|track| track.role == role)
                .expect("requested synthetic track")
                .path
                .clone()
        };
        let project_id = StudioProjectId::from_csprng([71; 16]).expect("project ID");
        let mut store =
            FilesystemStudioOriginalStore::new(directory.join("store")).expect("original store");
        let first_screen = commit_preview_video_asset(
            &mut store,
            project_id,
            &source_path(NativeStudioTrackRole::Screen),
            72,
            82,
            TrackKind::Screen,
            "screen-000.webm",
            quarter(0),
            quarter(4),
        );
        let second_screen = commit_preview_video_asset(
            &mut store,
            project_id,
            &source_path(NativeStudioTrackRole::Screen),
            73,
            83,
            TrackKind::Screen,
            "screen-001.webm",
            quarter(4),
            quarter(4),
        );
        let camera = commit_preview_video_asset(
            &mut store,
            project_id,
            &source_path(NativeStudioTrackRole::Camera),
            74,
            84,
            TrackKind::Camera,
            "camera-000.webm",
            quarter(0),
            quarter(8),
        );
        let system_audio = commit_preview_audio_asset(
            &mut store,
            project_id,
            &source_path(NativeStudioTrackRole::SystemAudio),
            75,
            85,
        );
        let edits = EditSpec {
            version: crate::STUDIO_EDIT_VERSION,
            revision: 1,
            operations: vec![EditOperation::AudioGain {
                track: TrackKind::SystemAudio,
                start: quarter(0),
                end: quarter(8),
                gain_millibels: -600,
                muted: false,
            }],
        };
        let project = StudioProjectManifest {
            version: crate::STUDIO_PROJECT_VERSION,
            id: project_id,
            revision: 1,
            state: StudioState::Editing,
            assets: vec![first_screen, second_screen.clone(), camera, system_audio],
            edits: edits.clone(),
        };
        let timeline = TimelineSource {
            duration: quarter(8),
            coverage: vec![
                SourceCoverage {
                    track: TrackKind::Screen,
                    start: quarter(0),
                    end: quarter(4),
                },
                SourceCoverage {
                    track: TrackKind::Screen,
                    start: quarter(4),
                    end: quarter(8),
                },
                SourceCoverage {
                    track: TrackKind::Camera,
                    start: quarter(0),
                    end: quarter(8),
                },
                SourceCoverage {
                    track: TrackKind::SystemAudio,
                    start: quarter(0),
                    end: quarter(8),
                },
            ],
            vfr_video_pts: BTreeMap::new(),
        };
        let plan = StudioTimelineCompiler::compile(&timeline, &edits).expect("preview plan");
        let sources = StudioSourceSet::from_project(&project, &timeline).expect("preview sources");
        let graph = StudioPreviewGraphSpec::compile(sources, plan).expect("preview graph");
        let second_screen_path = store
            .verified_original_path(project_id, &second_screen)
            .expect("verified second screen path");
        (graph, store, second_screen_path, second_screen.id)
    }

    #[test]
    fn segment_barrier_requires_every_matching_source_branch() {
        let expected = gst::Seqnum::next();
        let stale = gst::Seqnum::next();
        let mut state = SegmentCompletionState::new(expected, 2);

        assert!(!state.complete());
        state
            .observe_branch(BranchSegmentCompletion {
                branch: 0,
                seqnum: stale,
            })
            .expect("stale completion is ignored");
        assert!(!state.complete());
        state
            .observe_branch(BranchSegmentCompletion {
                branch: 0,
                seqnum: expected,
            })
            .expect("screen completion");
        assert!(!state.complete());
        state
            .observe_branch(BranchSegmentCompletion {
                branch: 1,
                seqnum: expected,
            })
            .expect("audio completion");
        assert!(state.complete());

        let mut invalid = SegmentCompletionState::new(expected, 1);
        assert!(matches!(
            invalid.observe_branch(BranchSegmentCompletion {
                branch: 1,
                seqnum: expected,
            }),
            Err(NativeExecutionError::InvalidGraph)
        ));
    }

    #[test]
    fn closing_video_hold_is_exactly_bounded() {
        assert_eq!(
            bounded_video_close_hold_ns(10, 10).expect("complete timeline"),
            None
        );
        assert_eq!(
            bounded_video_close_hold_ns(10, 10 + MAX_STUDIO_VIDEO_CLOSE_HOLD_NS)
                .expect("maximum approved hold"),
            Some(MAX_STUDIO_VIDEO_CLOSE_HOLD_NS)
        );
        assert!(matches!(
            bounded_video_close_hold_ns(10, 11 + MAX_STUDIO_VIDEO_CLOSE_HOLD_NS),
            Err(NativeExecutionError::InvalidOutput)
        ));
    }

    #[test]
    fn edited_preview_resolves_output_through_the_shared_executor() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let tracks = record_synthetic_studio_tracks(
            directory.path(),
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .expect("synthetic originals");
        let screen = tracks
            .iter()
            .find(|track| track.role == NativeStudioTrackRole::Screen)
            .expect("screen original");
        let plan = edited_plan();
        let preview = decode_studio_edited_preview_frame(
            &screen.path,
            &plan,
            ExactDuration::new(5, 8).expect("edited output position"),
            &CancellationToken::new(),
        )
        .expect("edited preview");
        assert_eq!(preview.plan_digest, plan.digest());
        assert_eq!(
            preview.resolved_source_time,
            ExactDuration::new(5, 4).expect("mapped source position")
        );
        assert_eq!((preview.frame.width, preview.frame.height), (320, 180));
    }

    #[test]
    fn preview_engine_owns_segmented_seek_playback_and_active_camera_decode() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let (graph, mut store, second_screen_path, second_screen_id) =
            preview_graph_fixture(directory.path());
        let plan_digest = graph.edit_plan_digest();
        let source_set_digest = graph.sources.digest();
        let mut engine = NativeStudioPreviewEngine::open(
            graph,
            &mut store,
            FrameRate {
                numerator: 30,
                denominator: 1,
            },
            &CancellationToken::new(),
        )
        .expect("open bounded preview engine");
        assert_eq!(
            engine.snapshot().state,
            NativeStudioPreviewPlaybackState::Paused
        );

        let seek_time = ExactDuration::new(5, 4).expect("segmented seek time");
        let sample = engine
            .seek(seek_time, &CancellationToken::new())
            .expect("decode exact seek");
        assert_eq!(sample.output_time, seek_time);
        assert_eq!(sample.source_time, seek_time);
        assert_eq!(sample.plan_digest, plan_digest);
        assert_eq!(sample.source_set_digest, source_set_digest);
        assert_eq!(sample.screen.source.asset_id, second_screen_id);
        assert_eq!(
            sample.screen.source.position,
            ExactDuration::new(1, 4).expect("asset-local position")
        );
        assert_eq!(
            (sample.screen.frame.width, sample.screen.frame.height),
            (320, 180)
        );
        assert_eq!(
            sample
                .camera
                .as_ref()
                .map(|camera| (camera.frame.width, camera.frame.height)),
            Some((320, 180))
        );
        assert!(sample.microphone.silent());
        assert!(!sample.system_audio.silent());
        assert_eq!(sample.system_audio.gain_millibels, -600);
        assert_eq!(
            sample.system_audio.source.map(|source| source.position),
            Some(seek_time)
        );

        engine.play().expect("start playback");
        let playing_generation = engine.snapshot().generation;
        let next = engine
            .next_frame(&CancellationToken::new())
            .expect("advance playback")
            .expect("one preview sample");
        assert_eq!(next.output_time, seek_time);
        assert_eq!(next.generation, playing_generation);
        assert_eq!(
            engine.snapshot().playhead,
            seek_time
                .checked_add(ExactDuration::new(1, 30).expect("frame period"))
                .expect("next playhead")
        );
        engine.pause().expect("pause playback");

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let before = engine.snapshot();
        assert!(matches!(
            engine.seek(ExactDuration::zero(), &cancelled),
            Err(NativeExecutionError::Cancelled)
        ));
        assert_eq!(engine.snapshot(), before);

        let displaced = second_screen_path.with_extension("displaced");
        fs::rename(&second_screen_path, &displaced).expect("displace immutable original");
        fs::copy(&displaced, &second_screen_path).expect("replace path with a distinct inode");
        assert!(matches!(
            engine.preview_current(&CancellationToken::new()),
            Err(NativeExecutionError::InvalidOutput)
        ));
    }

    #[test]
    fn edited_export_executes_timeline_and_audio_with_the_same_windows() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let tracks = record_synthetic_studio_tracks(
            directory.path(),
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .expect("synthetic originals");
        let path = |role| {
            tracks
                .iter()
                .find(|track| track.role == role)
                .expect("requested original")
                .path
                .clone()
        };
        let plan = edited_plan();
        let executor = StudioEditExecutor::compile(
            &plan,
            MAX_STUDIO_EDIT_EXECUTION_WINDOWS,
            &CancellationToken::new(),
        )
        .expect("shared windows");
        let output = directory.path().join("edited.webm");
        let mut progress = Vec::new();
        let aligned = NativeStudioAlignedSources {
            screen: path(NativeStudioTrackRole::Screen),
            microphone: None,
            system_audio: Some(path(NativeStudioTrackRole::SystemAudio)),
        };
        let artifact = render_studio_export_with_edits_and_progress(
            &aligned,
            &output,
            &plan,
            NativeStudioExportProfile::EditableWebM,
            &CancellationToken::new(),
            |event| progress.push(event),
        )
        .expect("edit-aware A/V export");
        assert_eq!(
            progress
                .first()
                .map(|event| (event.phase, event.basis_points)),
            Some((RenderPhase::Preparing, 0))
        );
        assert_eq!(
            progress
                .last()
                .map(|event| (event.phase, event.basis_points)),
            Some((RenderPhase::Finalizing, 10_000))
        );
        assert!(
            progress
                .windows(2)
                .all(|pair| pair[0].basis_points <= pair[1].basis_points
                    && render_phase_rank(pair[0].phase) <= render_phase_rank(pair[1].phase))
        );
        assert_eq!(
            progress
                .iter()
                .filter(|event| event.phase == RenderPhase::Encoding)
                .count(),
            executor.windows().len()
        );
        assert_eq!(artifact.plan_digest, plan.digest());
        assert_eq!(artifact.execution_windows, executor.windows().len());
        assert_eq!(artifact.audio_tracks, 1);
        assert_eq!(artifact.audio.map(|audio| audio.sample_rate), Some(48_000));
        assert_eq!(artifact.path, output);
        assert!(artifact.playable_container_marker);
        assert!(
            fs::read_dir(directory.path())
                .expect("output directory")
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".frame-studio-")),
            "successful publication must remove its private staging directory"
        );
        let output_duration_ns = media_duration(&artifact.path).nseconds();
        assert!(
            output_duration_ns.abs_diff(1_000_000_000) <= 100_000_000,
            "trim/delete/speed output duration {output_duration_ns}ns must remain within the approved frame/audio tolerance"
        );
        let decoded = decode_studio_preview_frame(
            &artifact.path,
            Duration::from_millis(500),
            &CancellationToken::new(),
        )
        .expect("edited output remains playable");
        assert_eq!((decoded.width, decoded.height), (320, 180));
        let preview = decode_studio_edited_preview_frame(
            &path(NativeStudioTrackRole::Screen),
            &plan,
            ExactDuration::new(1, 2).expect("half-second output position"),
            &CancellationToken::new(),
        )
        .expect("shared-plan preview");
        assert_eq!(preview.plan_digest, artifact.plan_digest);
        assert_eq!(
            (preview.frame.width, preview.frame.height),
            (decoded.width, decoded.height)
        );
        let (mean_error, peak_error) = rgb_error(&preview.frame.rgb, &decoded.rgb);
        assert!(
            mean_error <= 8 && peak_error <= 64,
            "decoded preview/export frame drift exceeded the golden tolerance: mean={mean_error}, peak={peak_error}"
        );

        let cancelled_output = directory.path().join("cancelled-after-progress.webm");
        let cancellation = CancellationToken::new();
        assert!(matches!(
            render_studio_export_with_edits_and_progress(
                &aligned,
                &cancelled_output,
                &plan,
                NativeStudioExportProfile::EditableWebM,
                &cancellation,
                |event| {
                    if event.phase == RenderPhase::Encoding {
                        cancellation.cancel();
                    }
                },
            ),
            Err(NativeExecutionError::Cancelled)
        ));
        assert!(!cancelled_output.exists());
        assert!(
            fs::read_dir(directory.path())
                .expect("output directory")
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".frame-studio-")),
            "cancelled publication must remove its private staging directory"
        );
    }

    fn rgb_error(expected: &[u8], actual: &[u8]) -> (u64, u8) {
        assert_eq!(expected.len(), actual.len(), "RGB frame shape");
        let mut total = 0_u64;
        let mut peak = 0_u8;
        for (&expected, &actual) in expected.iter().zip(actual) {
            let error = expected.abs_diff(actual);
            total = total.saturating_add(u64::from(error));
            peak = peak.max(error);
        }
        let mean = total / u64::try_from(expected.len()).expect("bounded frame length");
        (mean, peak)
    }

    #[test]
    fn approved_native_webm_and_archive_profiles_enforce_exact_output_caps() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let tracks = record_synthetic_studio_tracks(
            directory.path(),
            Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .expect("short synthetic originals");
        let path = |role| {
            tracks
                .iter()
                .find(|track| track.role == role)
                .expect("requested original")
                .path
                .clone()
        };
        let sources = NativeStudioAlignedSources {
            screen: path(NativeStudioTrackRole::Screen),
            microphone: None,
            system_audio: Some(path(NativeStudioTrackRole::SystemAudio)),
        };
        let plan = short_profile_plan(true);
        for (profile, name) in [
            (
                NativeStudioExportProfile::NativeHighQualityWebM,
                "native-high-quality.webm",
            ),
            (
                NativeStudioExportProfile::NativeArchiveMatroska,
                "native-archive.mkv",
            ),
        ] {
            let expected = ExportProfileSpec::approved(
                profile.approved_profile().expect("approved native profile"),
            );
            let artifact = render_studio_export_with_edits(
                &sources,
                &directory.path().join(name),
                &plan,
                profile,
                &CancellationToken::new(),
            )
            .expect("exact approved native profile");
            assert_eq!(artifact.approved_profile, Some(expected));
            assert_eq!(artifact.video.width, expected.resolution.width);
            assert_eq!(artifact.video.height, expected.resolution.height);
            assert_eq!(artifact.video.frame_rate, expected.frame_rate);
            assert_eq!(
                artifact.audio,
                Some(NativeStudioOutputAudio {
                    sample_rate: 48_000,
                    channels: 2,
                })
            );
            assert!(artifact.bytes > 128);
        }
    }

    #[test]
    fn approved_licensed_profiles_fail_closed_or_emit_exact_caps() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let sources = NativeStudioAlignedSources {
            screen: directory.path().join("unused-screen.webm"),
            microphone: None,
            system_audio: Some(directory.path().join("unused-system-audio.webm")),
        };
        let plan = short_profile_plan(true);
        assert!(matches!(
            render_studio_export_with_edits(
                &NativeStudioAlignedSources {
                    screen: directory.path().join("unused-screen.webm"),
                    microphone: None,
                    system_audio: None,
                },
                &directory.path().join("audio-less-native-high-quality.webm"),
                &short_profile_plan(false),
                NativeStudioExportProfile::NativeHighQualityWebM,
                &CancellationToken::new(),
            ),
            Err(NativeExecutionError::InvalidGraph)
        ));
        if std::env::var("FRAME_NATIVE_H264_AAC_APPROVED").as_deref() != Ok("approved-v1") {
            assert!(matches!(
                render_studio_export_with_edits(
                    &sources,
                    &directory.path().join("distribution-master.mp4"),
                    &plan,
                    NativeStudioExportProfile::DistributionMasterMp4,
                    &CancellationToken::new(),
                ),
                Err(NativeExecutionError::CodecApprovalRequired)
            ));
        }
        if std::env::var("FRAME_NATIVE_HEVC_AAC_APPROVED").as_deref() != Ok("approved-v1") {
            assert!(matches!(
                render_studio_export_with_edits(
                    &sources,
                    &directory.path().join("native-high-quality-hevc.mp4"),
                    &plan,
                    NativeStudioExportProfile::NativeHighQualityHevcMp4,
                    &CancellationToken::new(),
                ),
                Err(NativeExecutionError::CodecApprovalRequired)
            ));
        }
    }

    #[test]
    fn export_postcondition_probe_forces_the_software_decoder_path() {
        prepare_runtime().expect("trusted runtime");
        let player = software_output_probe_player().expect("software output probe");
        assert_eq!(
            player.factory().expect("player factory").name().as_str(),
            "playbin"
        );
        let flags = player.property_value("flags");
        let (_, values) =
            gst::glib::FlagsValue::from_value(&flags).expect("playbin flags property");
        assert!(
            values
                .iter()
                .any(|value| value.nick() == "force-sw-decoders")
        );
    }

    #[test]
    fn approved_distribution_profile_emits_exact_cloudflare_input_caps() {
        if std::env::var("FRAME_NATIVE_H264_AAC_APPROVED").as_deref() != Ok("approved-v1") {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let tracks = record_synthetic_studio_tracks(
            directory.path(),
            Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .expect("short synthetic originals");
        let path = |role| {
            tracks
                .iter()
                .find(|track| track.role == role)
                .expect("requested original")
                .path
                .clone()
        };
        let profile = NativeStudioExportProfile::DistributionMasterMp4;
        let expected = ExportProfileSpec::approved(
            profile
                .approved_profile()
                .expect("approved distribution profile"),
        );
        let output = directory.path().join("distribution-master.mp4");
        let artifact = render_studio_export_with_edits(
            &NativeStudioAlignedSources {
                screen: path(NativeStudioTrackRole::Screen),
                microphone: None,
                system_audio: Some(path(NativeStudioTrackRole::SystemAudio)),
            },
            &output,
            &short_profile_plan(true),
            profile,
            &CancellationToken::new(),
        )
        .expect("exact Cloudflare-compatible distribution master");
        assert_eq!(artifact.approved_profile, Some(expected));
        assert_eq!(
            (artifact.video.width, artifact.video.height),
            (1_920, 1_080)
        );
        assert_eq!(
            artifact.video.frame_rate,
            FrameRate {
                numerator: 30,
                denominator: 1,
            }
        );
        assert_eq!(artifact.video.colorimetry, "2:3:5:1");
        assert_eq!(
            artifact.audio,
            Some(NativeStudioOutputAudio {
                sample_rate: 48_000,
                channels: 2,
            })
        );
    }

    #[test]
    fn approved_hevc_profile_emits_exact_native_high_quality_caps() {
        if std::env::var("FRAME_NATIVE_HEVC_AAC_APPROVED").as_deref() != Ok("approved-v1") {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let tracks = record_synthetic_studio_tracks(
            directory.path(),
            Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .expect("short synthetic originals");
        let path = |role| {
            tracks
                .iter()
                .find(|track| track.role == role)
                .expect("requested original")
                .path
                .clone()
        };
        let profile = NativeStudioExportProfile::NativeHighQualityHevcMp4;
        let expected =
            ExportProfileSpec::approved(profile.approved_profile().expect("approved HEVC profile"));
        let artifact = render_studio_export_with_edits(
            &NativeStudioAlignedSources {
                screen: path(NativeStudioTrackRole::Screen),
                microphone: None,
                system_audio: Some(path(NativeStudioTrackRole::SystemAudio)),
            },
            &directory.path().join("native-high-quality-hevc.mp4"),
            &profile_plan(3, true),
            profile,
            &CancellationToken::new(),
        )
        .expect("exact native high-quality HEVC artifact");
        assert_eq!(artifact.approved_profile, Some(expected));
        assert_eq!(
            (artifact.video.width, artifact.video.height),
            (3_840, 2_160)
        );
        assert_eq!(
            artifact.video.frame_rate,
            FrameRate {
                numerator: 60,
                denominator: 1,
            }
        );
        assert_eq!(artifact.video.colorimetry, "1:3:7:11");
        assert_eq!(
            artifact.audio,
            Some(NativeStudioOutputAudio {
                sample_rate: 48_000,
                channels: 2,
            })
        );
    }

    #[test]
    fn unsupported_visual_composition_and_pre_cancel_fail_closed() {
        let plan = edited_plan();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            decode_studio_edited_preview_frame(
                Path::new("/unused"),
                &plan,
                ExactDuration::zero(),
                &cancellation,
            ),
            Err(NativeExecutionError::Cancelled)
        ));
        let directory = tempfile::tempdir().expect("temporary directory");
        let cancelled_output = directory.path().join("cancelled.webm");
        assert!(matches!(
            render_studio_export_with_edits(
                &NativeStudioAlignedSources {
                    screen: PathBuf::from("unused-screen.webm"),
                    microphone: None,
                    system_audio: Some(PathBuf::from("unused-audio.webm")),
                },
                &cancelled_output,
                &plan,
                NativeStudioExportProfile::EditableWebM,
                &cancellation,
            ),
            Err(NativeExecutionError::Cancelled)
        ));
        assert!(!cancelled_output.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let dangling = directory.path().join("dangling.webm");
            symlink(directory.path().join("missing-target"), &dangling)
                .expect("dangling output symlink");
            assert!(matches!(
                StudioOutputReservation::new(dangling, NativeStudioExportProfile::EditableWebM,),
                Err(NativeExecutionError::InvalidOutput)
            ));
        }
        assert!(matches!(
            validate_supported_composition(
                &[StudioEditExecutionWindow {
                    style: crate::CompositeStyle {
                        layout: LayoutPreset::CameraFull,
                        ..crate::CompositeStyle::default()
                    },
                    ..StudioEditExecutor::compile(
                        &plan,
                        MAX_STUDIO_EDIT_EXECUTION_WINDOWS,
                        &CancellationToken::new(),
                    )
                    .expect("windows")
                    .windows()[0]
                }],
                &NativeStudioAlignedSources {
                    screen: PathBuf::from("screen.webm"),
                    microphone: None,
                    system_audio: Some(PathBuf::from("audio.webm")),
                },
            ),
            Err(NativeExecutionError::InvalidGraph)
        ));
    }
}
