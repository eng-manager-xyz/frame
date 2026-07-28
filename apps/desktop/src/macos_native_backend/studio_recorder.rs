//! Desktop ownership boundary for durable native Studio originals.
//!
//! The macOS worker remains the sole owner of native capture. This adapter
//! gives that worker one bounded route into the provider-neutral Studio
//! encoder and filesystem session without exposing raw media through IPC.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use frame_media::{
    AudioSampleFormat, BoundedMediaQueue, CancellationToken, CaptureElementFamily,
    DurableStudioJournal, EditSpec, FilesystemStudioJournalStore, FilesystemStudioOriginalStore,
    FilesystemStudioProjectStore, FilesystemStudioRecordingSession, FrameRate, FrameTimestamp,
    IsolatedTrackBranch, JournalAdvanceRequest, JournalBoundary, MAX_STUDIO_DOCUMENT_BYTES,
    NativeStudioInputBuffer, NativeStudioRecording, NativeStudioRecordingArtifact,
    NativeStudioRecordingError, PendingAssetCommit, PixelFormat, ReceiptKind,
    STUDIO_JOURNAL_VERSION, STUDIO_PROJECT_VERSION, StudioAsset, StudioAssetEncoding,
    StudioAssetId, StudioAudioRawCaps, StudioDocumentCodec, StudioJournalSnapshot,
    StudioOperationId, StudioProjectId, StudioProjectManifest, StudioRecordingGraphSpec,
    StudioSourceName, StudioState, StudioVideoRawCaps, StudioWorkerId, TempAssetCommitTicket,
    TrackKind, VideoFrameSpec, commit_verified_temporary, strong_sha256,
};
use ring::rand::{SecureRandom, SystemRandom};

use crate::{NativeDesktopBackendError, native_screen_worker::StudioProjectArtifact};

const STUDIO_QUEUE_BUFFERS: u32 = 64;
const STUDIO_QUEUE_BYTES: u64 = 256 * 1024 * 1024;
const STUDIO_QUEUE_TIME_NS: u64 = 2_000_000_000;
const MAXIMUM_STUDIO_TRACK_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const INITIAL_JOURNAL_REVISION: u64 = 1;
const INITIAL_JOURNAL_FENCE: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StudioRecordingIdentity {
    pub(super) project: [u8; 16],
    pub(super) clock: [u8; 16],
    pub(super) screen_asset: [u8; 16],
    pub(super) microphone_asset: [u8; 16],
    pub(super) system_audio_asset: [u8; 16],
    pub(super) camera_asset: [u8; 16],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct StudioOptionalTracks {
    pub(super) microphone: bool,
    pub(super) system_audio: bool,
    pub(super) camera: bool,
}

#[derive(Debug)]
pub(super) struct DesktopStudioRecordingArtifact {
    pub(super) project: StudioProjectArtifact,
}

#[derive(Debug)]
pub(super) struct DesktopStudioFinishFailure {
    pub(super) error: NativeDesktopBackendError,
    /// Persistence runs only after the GStreamer graph reaches `Null`.
    /// Native finish failures remain conservative because the underlying
    /// execution error does not yet distinguish every teardown site.
    pub(super) teardown_confirmed: bool,
}

/// One active, isolated-track Studio encoder and its durable session.
pub(super) struct DesktopStudioRecording {
    inner: NativeStudioRecording,
    journal: DurableStudioJournal<FilesystemStudioJournalStore>,
    originals: FilesystemStudioOriginalStore,
    projects_root: PathBuf,
    project_id: StudioProjectId,
    project_identity: [u8; 16],
}

impl std::fmt::Debug for DesktopStudioRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopStudioRecording")
            .field("inner", &self.inner)
            .field("journal", &self.journal)
            .field("originals", &self.originals)
            .field("projects_root", &"<redacted>")
            .field("project_id", &self.project_id)
            .finish()
    }
}

impl DesktopStudioRecording {
    pub(super) fn start(
        originals_root: &Path,
        projects_root: &Path,
        identity: StudioRecordingIdentity,
        screen: VideoFrameSpec,
        frame_rate: u16,
        optional: StudioOptionalTracks,
    ) -> Result<Self, NativeDesktopBackendError> {
        let graph = recording_graph(identity, screen, frame_rate, optional)?;
        let project_id = graph.project_id;
        let owner = random_worker_id()?;
        let journal_store =
            FilesystemStudioJournalStore::new(projects_root).map_err(map_studio_error)?;
        let mut journal = DurableStudioJournal::create(
            journal_store,
            StudioJournalSnapshot {
                version: STUDIO_JOURNAL_VERSION,
                project_id,
                revision: INITIAL_JOURNAL_REVISION,
                fence: INITIAL_JOURNAL_FENCE,
                owner,
                boundary: JournalBoundary::Created,
                last_operation_id: None,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipts: BTreeMap::new(),
            },
        )
        .map_err(map_studio_error)?;
        let originals =
            FilesystemStudioOriginalStore::new(originals_root).map_err(map_studio_error)?;
        let session = FilesystemStudioRecordingSession::begin(
            &originals,
            graph.clone(),
            MAXIMUM_STUDIO_TRACK_BYTES,
        )
        .map_err(map_studio_error)?;
        advance_journal(
            &mut journal,
            identity.project,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
            None,
        )?;
        let inner = NativeStudioRecording::start(&graph, session).map_err(map_native_error)?;
        Ok(Self {
            inner,
            journal,
            originals,
            projects_root: projects_root.to_path_buf(),
            project_id,
            project_identity: identity.project,
        })
    }

    /// Record the durable boundary only after the owning native capture source
    /// has started. Callers must abort both native authorities if this fails.
    pub(super) fn capture_started(&mut self) -> Result<(), NativeDesktopBackendError> {
        advance_journal(
            &mut self.journal,
            self.project_identity,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
            None,
        )?;
        Ok(())
    }

    pub(super) fn push_screen(
        &mut self,
        sequence: u64,
        timestamp: FrameTimestamp,
        pixels: Vec<u8>,
    ) -> Result<(), NativeDesktopBackendError> {
        self.push(TrackKind::Screen, sequence, timestamp, pixels)
    }

    pub(super) fn push_system_audio(
        &mut self,
        sequence: u64,
        timestamp: FrameTimestamp,
        samples: Vec<u8>,
    ) -> Result<(), NativeDesktopBackendError> {
        self.push(TrackKind::SystemAudio, sequence, timestamp, samples)
    }

    fn push(
        &mut self,
        track: TrackKind,
        sequence: u64,
        timestamp: FrameTimestamp,
        bytes: Vec<u8>,
    ) -> Result<(), NativeDesktopBackendError> {
        let input = NativeStudioInputBuffer::new(track, sequence, timestamp, bytes)
            .map_err(map_native_error)?;
        self.inner.push(input).map_err(map_native_error)?;
        Ok(())
    }

    pub(super) fn finish(
        self,
    ) -> Result<DesktopStudioRecordingArtifact, DesktopStudioFinishFailure> {
        let Self {
            inner,
            mut journal,
            mut originals,
            projects_root,
            project_id,
            project_identity,
        } = self;
        let recording = inner.finish(&CancellationToken::new()).map_err(|error| {
            DesktopStudioFinishFailure {
                error: map_native_error(error),
                teardown_confirmed: false,
            }
        })?;
        let project = persist_completion(
            &mut journal,
            &mut originals,
            &projects_root,
            project_id,
            project_identity,
            &recording,
        )
        .map_err(|error| DesktopStudioFinishFailure {
            error,
            teardown_confirmed: true,
        })?;
        Ok(DesktopStudioRecordingArtifact { project })
    }

    pub(super) fn abort(self) -> Result<(), NativeDesktopBackendError> {
        self.inner.abort().map_err(map_native_error)
    }
}

fn persist_completion(
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    originals: &mut FilesystemStudioOriginalStore,
    projects_root: &Path,
    project_id: StudioProjectId,
    project_identity: [u8; 16],
    recording: &NativeStudioRecordingArtifact,
) -> Result<StudioProjectArtifact, NativeDesktopBackendError> {
    let mut durable_assets = Vec::with_capacity(recording.assets.len());
    for temporary in &recording.assets {
        advance_journal(
            journal,
            project_identity,
            JournalBoundary::TempAssetReserved,
            ReceiptKind::TempReserved,
            Some(temporary),
        )?;
        advance_journal(
            journal,
            project_identity,
            JournalBoundary::TempAssetDurable,
            ReceiptKind::TempDurable,
            Some(temporary),
        )?;
        let commit_operation = advance_journal(
            journal,
            project_identity,
            JournalBoundary::AssetCommitRequested,
            ReceiptKind::AssetCommitRequested,
            Some(temporary),
        )?;
        let durable = commit_verified_temporary(
            originals,
            TempAssetCommitTicket::new(
                project_id,
                commit_operation,
                journal.snapshot().fence,
                temporary.clone(),
            )
            .map_err(map_studio_error)?,
        )
        .map_err(map_studio_error)?;
        advance_journal(
            journal,
            project_identity,
            JournalBoundary::AssetCommitted,
            ReceiptKind::AssetCommitted,
            Some(&durable),
        )?;
        advance_journal(
            journal,
            project_identity,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
            None,
        )?;
        durable_assets.push(durable);
    }

    let manifest = StudioProjectManifest {
        version: STUDIO_PROJECT_VERSION,
        id: project_id,
        revision: 1,
        state: StudioState::Editing,
        assets: durable_assets,
        edits: EditSpec::default(),
    };
    let mut projects = FilesystemStudioProjectStore::new(projects_root, journal.snapshot().fence)
        .map_err(map_studio_error)?;
    projects
        .create_project(&manifest)
        .map_err(map_studio_error)?;
    let path = projects
        .verified_project_path(project_id)
        .map_err(map_studio_error)?;
    let project = authenticate_project_file(path)?;
    advance_journal(
        journal,
        project_identity,
        JournalBoundary::RecordingStopped,
        ReceiptKind::RecordingStopped,
        None,
    )?;
    Ok(project)
}

fn authenticate_project_file(
    path: PathBuf,
) -> Result<StudioProjectArtifact, NativeDesktopBackendError> {
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_STUDIO_DOCUMENT_BYTES as u64
    {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    let bytes = fs::read(&path).map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if bytes.len() as u64 != metadata.len() || bytes.len() > MAX_STUDIO_DOCUMENT_BYTES {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    StudioDocumentCodec::decode_project(&bytes).map_err(map_studio_error)?;
    Ok(StudioProjectArtifact {
        path,
        bytes: metadata.len(),
        sha256: strong_sha256(&bytes).to_hex(),
    })
}

fn advance_journal(
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    project_identity: [u8; 16],
    boundary: JournalBoundary,
    receipt_kind: ReceiptKind,
    asset: Option<&StudioAsset>,
) -> Result<StudioOperationId, NativeDesktopBackendError> {
    let operation_id = random_operation_id()?;
    let pending_asset = asset.cloned().map(|asset| PendingAssetCommit {
        operation_id,
        asset,
    });
    let command_digest = lifecycle_digest(
        b"frame-desktop-studio-journal-command-v1",
        project_identity,
        boundary,
        asset,
    )?;
    let outcome_digest = lifecycle_digest(
        b"frame-desktop-studio-journal-outcome-v1",
        project_identity,
        boundary,
        asset,
    )?;
    journal
        .advance(JournalAdvanceRequest {
            expected_revision: journal.snapshot().revision,
            expected_fence: journal.snapshot().fence,
            operation_id,
            command_digest,
            boundary,
            pending_asset,
            pending_edit: None,
            pending_render: None,
            receipt_kind,
            outcome_digest,
        })
        .map_err(map_studio_error)?;
    Ok(operation_id)
}

fn lifecycle_digest(
    domain: &[u8],
    project_identity: [u8; 16],
    boundary: JournalBoundary,
    asset: Option<&StudioAsset>,
) -> Result<frame_media::Sha256Digest, NativeDesktopBackendError> {
    let mut canonical = Vec::with_capacity(512);
    canonical.extend_from_slice(domain);
    canonical.push(0);
    canonical.extend_from_slice(&project_identity);
    canonical.push(boundary.canonical_tag());
    if let Some(asset) = asset {
        canonical.extend_from_slice(
            &StudioDocumentCodec::encode_asset(asset).map_err(map_studio_error)?,
        );
    }
    Ok(strong_sha256(&canonical))
}

fn random_operation_id() -> Result<StudioOperationId, NativeDesktopBackendError> {
    StudioOperationId::from_csprng(random_identity()?).map_err(map_studio_error)
}

fn random_worker_id() -> Result<StudioWorkerId, NativeDesktopBackendError> {
    StudioWorkerId::from_csprng(random_identity()?).map_err(map_studio_error)
}

fn random_identity() -> Result<[u8; 16], NativeDesktopBackendError> {
    let mut bytes = [0_u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| NativeDesktopBackendError::Internal)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(NativeDesktopBackendError::Internal);
    }
    Ok(bytes)
}

fn recording_graph(
    identity: StudioRecordingIdentity,
    screen: VideoFrameSpec,
    frame_rate: u16,
    optional: StudioOptionalTracks,
) -> Result<StudioRecordingGraphSpec, NativeDesktopBackendError> {
    let project = StudioProjectId::from_csprng(identity.project).map_err(map_studio_error)?;
    let clock = StudioOperationId::from_csprng(identity.clock).map_err(map_studio_error)?;
    let mut branches = vec![video_branch(
        TrackKind::Screen,
        identity.screen_asset,
        "screen.webm",
        screen.width,
        screen.height,
        frame_rate,
    )?];
    if optional.microphone {
        branches.push(audio_branch(
            TrackKind::Microphone,
            identity.microphone_asset,
            "microphone.webm",
        )?);
    }
    if optional.system_audio {
        branches.push(audio_branch(
            TrackKind::SystemAudio,
            identity.system_audio_asset,
            "system-audio.webm",
        )?);
    }
    if optional.camera {
        // The combined native bridge negotiates the canonical camera format.
        branches.push(video_branch(
            TrackKind::Camera,
            identity.camera_asset,
            "camera.webm",
            1_280,
            720,
            30,
        )?);
    }
    StudioRecordingGraphSpec::new(project, clock, branches).map_err(map_studio_error)
}

fn video_branch(
    track: TrackKind,
    id: [u8; 16],
    name: &str,
    width: u32,
    height: u32,
    frame_rate: u16,
) -> Result<IsolatedTrackBranch, NativeDesktopBackendError> {
    let encoding = StudioAssetEncoding::recording_vp8_webm(StudioVideoRawCaps {
        width,
        height,
        frame_rate: FrameRate {
            numerator: u32::from(frame_rate),
            denominator: 1,
        },
        pixel_format: PixelFormat::Bgra8,
    })
    .map_err(map_studio_error)?;
    branch(track, id, name, CaptureElementFamily::Vp8Encoder, encoding)
}

fn audio_branch(
    track: TrackKind,
    id: [u8; 16],
    name: &str,
) -> Result<IsolatedTrackBranch, NativeDesktopBackendError> {
    let encoding = StudioAssetEncoding::recording_opus_webm(StudioAudioRawCaps {
        sample_rate: 48_000,
        channels: 2,
        sample_format: AudioSampleFormat::Float32,
    })
    .map_err(map_studio_error)?;
    branch(track, id, name, CaptureElementFamily::OpusEncoder, encoding)
}

fn branch(
    track: TrackKind,
    id: [u8; 16],
    name: &str,
    encoder: CaptureElementFamily,
    encoding: StudioAssetEncoding,
) -> Result<IsolatedTrackBranch, NativeDesktopBackendError> {
    let source = match track {
        TrackKind::Screen => CaptureElementFamily::NativeScreenBridge,
        TrackKind::Camera => CaptureElementFamily::NativeCameraBridge,
        TrackKind::Microphone => CaptureElementFamily::NativeMicrophoneBridge,
        TrackKind::SystemAudio => CaptureElementFamily::NativeSystemAudioBridge,
    };
    Ok(IsolatedTrackBranch {
        track,
        asset_id: StudioAssetId::from_csprng(id).map_err(map_studio_error)?,
        temporary_name: StudioSourceName::new(name).map_err(map_studio_error)?,
        source,
        encoder,
        muxer: CaptureElementFamily::WebMMux,
        encoding,
        queue: BoundedMediaQueue {
            max_buffers: STUDIO_QUEUE_BUFFERS,
            max_bytes: STUDIO_QUEUE_BYTES,
            max_time_ns: STUDIO_QUEUE_TIME_NS,
        },
    })
}

fn map_studio_error(error: frame_media::StudioError) -> NativeDesktopBackendError {
    match error {
        frame_media::StudioError::StorageIo | frame_media::StudioError::UnsafeStoragePath => {
            NativeDesktopBackendError::Filesystem
        }
        _ => NativeDesktopBackendError::Internal,
    }
}

fn map_native_error(error: NativeStudioRecordingError) -> NativeDesktopBackendError {
    match error {
        NativeStudioRecordingError::Studio(frame_media::StudioError::StorageIo)
        | NativeStudioRecordingError::Studio(frame_media::StudioError::UnsafeStoragePath) => {
            NativeDesktopBackendError::Filesystem
        }
        _ => NativeDesktopBackendError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use frame_media::{ColorSpace, FrameMemory};

    use super::*;
    use crate::macos_native_backend::studio_projects::{authenticate_ready, discover};
    use crate::{NativeStudioProjectStatus, RootedDir};

    const VIDEO_DURATION_NS: u64 = 33_333_333;
    const AUDIO_DURATION_NS: u64 = 21_333_333;

    fn identity() -> StudioRecordingIdentity {
        StudioRecordingIdentity {
            project: [1; 16],
            clock: [2; 16],
            screen_asset: [3; 16],
            microphone_asset: [4; 16],
            system_audio_asset: [5; 16],
            camera_asset: [6; 16],
        }
    }

    fn screen_spec() -> VideoFrameSpec {
        VideoFrameSpec {
            width: 160,
            height: 90,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: VIDEO_DURATION_NS,
            memory: FrameMemory::Cpu,
        }
    }

    fn audio(sequence: u64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1_024 * 2 * size_of::<f32>());
        for frame in 0..1_024 {
            let sample = ((frame as f32 / 1_024.0) * TAU).sin() * 0.25;
            bytes.extend_from_slice(&sample.to_le_bytes());
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        assert!(sequence > 0);
        bytes
    }

    #[test]
    fn production_adapter_seals_screen_and_system_audio_originals() {
        let directory = tempfile::tempdir().expect("Studio root");
        let originals = directory.path().join("originals");
        let projects = directory.path().join("projects");
        let mut recording = DesktopStudioRecording::start(
            &originals,
            &projects,
            identity(),
            screen_spec(),
            30,
            StudioOptionalTracks {
                system_audio: true,
                ..StudioOptionalTracks::default()
            },
        )
        .expect("Studio recording");
        recording
            .capture_started()
            .expect("durable capture-start boundary");
        for sequence in 1..=30 {
            recording
                .push_screen(
                    sequence,
                    FrameTimestamp::new((sequence - 1) * VIDEO_DURATION_NS, VIDEO_DURATION_NS)
                        .expect("video timestamp"),
                    vec![42; 160 * 90 * 4],
                )
                .expect("screen original");
        }
        for sequence in 1..=47 {
            recording
                .push_system_audio(
                    sequence,
                    FrameTimestamp::new((sequence - 1) * AUDIO_DURATION_NS, AUDIO_DURATION_NS)
                        .expect("audio timestamp"),
                    audio(sequence),
                )
                .expect("system-audio original");
        }
        let artifact = recording.finish().expect("sealed Studio originals");
        assert!(
            artifact
                .project
                .path
                .starts_with(fs::canonicalize(&projects).expect("canonical projects root"))
        );
        assert!(artifact.project.bytes > 0);
        let project_bytes = fs::read(&artifact.project.path).expect("canonical Studio project");
        assert_eq!(
            artifact.project.sha256,
            strong_sha256(&project_bytes).to_hex()
        );
        let project =
            StudioDocumentCodec::decode_project(&project_bytes).expect("valid Studio project");
        assert_eq!(project.state, StudioState::Editing);
        assert_eq!(project.assets.len(), 2);
        assert!(
            project
                .assets
                .iter()
                .all(|asset| asset.commit_state == frame_media::AssetCommitState::DurableOriginal)
        );
        let journal = DurableStudioJournal::open(
            FilesystemStudioJournalStore::new(&projects).expect("journal store"),
            StudioProjectId::from_csprng(identity().project).expect("project id"),
        )
        .expect("durable Studio journal");
        assert_eq!(
            journal.snapshot().boundary,
            JournalBoundary::RecordingStopped
        );
        let canonical_projects = fs::canonicalize(&projects).expect("canonical projects root");
        let projects_directory =
            RootedDir::bind(&canonical_projects).expect("pinned projects root");
        let discovered = discover(&projects_directory).expect("discover Studio project");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].status(), NativeStudioProjectStatus::Ready);
        assert_eq!(discovered[0].revision(), Some(1));
        let (revision, duration_ms) =
            authenticate_ready(&projects_directory, &discovered[0]).expect("open ready project");
        assert_eq!(revision, 1);
        assert_eq!(duration_ms, 1_003);
    }

    #[test]
    fn production_adapter_keeps_optional_tracks_optional() {
        let directory = tempfile::tempdir().expect("Studio root");
        let originals = directory.path().join("originals");
        let projects = directory.path().join("projects");
        let mut recording = DesktopStudioRecording::start(
            &originals,
            &projects,
            identity(),
            screen_spec(),
            30,
            StudioOptionalTracks::default(),
        )
        .expect("screen-only Studio recording");
        recording
            .capture_started()
            .expect("durable capture-start boundary");
        for sequence in 1..=6 {
            recording
                .push_screen(
                    sequence,
                    FrameTimestamp::new((sequence - 1) * VIDEO_DURATION_NS, VIDEO_DURATION_NS)
                        .expect("video timestamp"),
                    vec![7; 160 * 90 * 4],
                )
                .expect("screen original");
        }
        let artifact = recording.finish().expect("sealed screen original");
        let project = StudioDocumentCodec::decode_project(
            &fs::read(artifact.project.path).expect("Studio project"),
        )
        .expect("valid project");
        assert_eq!(project.assets.len(), 1);
        assert_eq!(project.assets[0].track, TrackKind::Screen);
    }

    #[test]
    fn capture_start_boundary_survives_an_aborted_native_session() {
        let directory = tempfile::tempdir().expect("Studio root");
        let originals = directory.path().join("originals");
        let projects = directory.path().join("projects");
        let mut recording = DesktopStudioRecording::start(
            &originals,
            &projects,
            identity(),
            screen_spec(),
            30,
            StudioOptionalTracks::default(),
        )
        .expect("screen-only Studio recording");
        recording
            .capture_started()
            .expect("durable capture-start boundary");
        recording.abort().expect("confirmed graph teardown");

        let journal = DurableStudioJournal::open(
            FilesystemStudioJournalStore::new(&projects).expect("journal store"),
            StudioProjectId::from_csprng(identity().project).expect("project id"),
        )
        .expect("recoverable journal");
        assert_eq!(journal.snapshot().boundary, JournalBoundary::CaptureStarted);
        let canonical_projects = fs::canonicalize(&projects).expect("canonical projects root");
        let projects_directory =
            RootedDir::bind(&canonical_projects).expect("pinned projects root");
        let discovered = discover(&projects_directory).expect("discover recoverable project");
        assert_eq!(discovered.len(), 1);
        assert_eq!(
            discovered[0].status(),
            NativeStudioProjectStatus::RecoveryRequired
        );
        assert_eq!(discovered[0].revision(), None);
        assert!(authenticate_ready(&projects_directory, &discovered[0]).is_err());
    }
}
