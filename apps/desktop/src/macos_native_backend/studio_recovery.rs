//! Journal-fenced production recovery for native Studio projects.
//!
//! Recovery takes ownership before touching media, reconstructs the exact
//! persisted graph, reconciles immutable originals and edit saves, and never
//! accepts a filesystem path or durable identity from the WebView.

use std::path::Path;

use frame_media::{
    AssetCommitState, CancellationToken, DurableStudioJournal, EditSaveTicket,
    FilesystemStudioJournalStore, FilesystemStudioOriginalStore, FilesystemStudioProjectStore,
    FilesystemStudioRecordingSession, JournalAdvanceRequest, JournalBoundary, PendingEditSave,
    RationalTime, ReceiptKind, STUDIO_PROJECT_VERSION, StudioAsset, StudioDocumentCodec,
    StudioError, StudioProjectManifest, StudioProjectStorePort, StudioState, TempAssetCommitTicket,
    TrackKind, commit_edit_save, commit_verified_temporary, probe_studio_media_duration,
    strong_sha256,
};

use super::{
    studio_projects::{DiscoveredStudioProject, archive_current_journal},
    studio_recorder::{advance_journal, map_studio_error, random_operation_id, random_worker_id},
};
use crate::{NativeDesktopBackendError, NativeStudioRecoveryAction, RootedDir};

pub(super) fn recover_project(
    projects_root: &Path,
    originals_root: &Path,
    discovered: &DiscoveredStudioProject,
) -> Result<StudioProjectManifest, NativeDesktopBackendError> {
    let cancellation = CancellationToken::new();
    recover_project_with_probe(
        projects_root,
        originals_root,
        discovered,
        &mut |_track, path| {
            probe_studio_media_duration(path, &cancellation).map_err(|error| match error {
                frame_media::NativeExecutionError::Filesystem => StudioError::StorageIo,
                _ => StudioError::InvalidAsset,
            })
        },
    )
}

fn recover_project_with_probe(
    projects_root: &Path,
    originals_root: &Path,
    discovered: &DiscoveredStudioProject,
    probe_duration: &mut dyn FnMut(TrackKind, &Path) -> Result<RationalTime, StudioError>,
) -> Result<StudioProjectManifest, NativeDesktopBackendError> {
    let store = FilesystemStudioJournalStore::new(projects_root).map_err(map_studio_error)?;
    let mut journal =
        DurableStudioJournal::open(store, discovered.project_id()).map_err(map_studio_error)?;
    if journal.snapshot() != discovered.journal() {
        return Err(NativeDesktopBackendError::StaleCatalog);
    }
    take_recovery_ownership(&mut journal)?;
    match discovered.recovery_action() {
        NativeStudioRecoveryAction::RecoverRecording => recover_recording(
            projects_root,
            originals_root,
            discovered.project_identity(),
            &mut journal,
            probe_duration,
        ),
        NativeStudioRecoveryAction::ReconcileEditSave => {
            recover_edit_save(projects_root, discovered.project_identity(), &mut journal)
        }
        NativeStudioRecoveryAction::ArchiveUnstartedAttempt
        | NativeStudioRecoveryAction::OpenEditor
        | NativeStudioRecoveryAction::RequiresOperatorDecision => {
            Err(NativeDesktopBackendError::Unavailable)
        }
    }
}

pub(super) fn archive_unstarted_project(
    projects_root: &Path,
    originals_root: &Path,
    projects_directory: &RootedDir,
    discovered: &DiscoveredStudioProject,
) -> Result<(), NativeDesktopBackendError> {
    if discovered.recovery_action() != NativeStudioRecoveryAction::ArchiveUnstartedAttempt {
        return Err(NativeDesktopBackendError::Unavailable);
    }
    let store = FilesystemStudioJournalStore::new(projects_root).map_err(map_studio_error)?;
    let mut journal =
        DurableStudioJournal::open(store, discovered.project_id()).map_err(map_studio_error)?;
    if journal.snapshot() != discovered.journal() {
        return Err(NativeDesktopBackendError::StaleCatalog);
    }
    take_recovery_ownership(&mut journal)?;
    let mut originals =
        FilesystemStudioOriginalStore::new(originals_root).map_err(map_studio_error)?;
    let graph = originals
        .find_persisted_recording_graph(discovered.project_id())
        .map_err(map_studio_error)?;
    match (journal.snapshot().boundary, graph) {
        (JournalBoundary::Created, None) => {}
        (JournalBoundary::Created | JournalBoundary::RecordingGraphPrepared, Some(document)) => {
            let graph = document.graph;
            let session = FilesystemStudioRecordingSession::recover_reconciling_originals(
                &mut originals,
                graph.clone(),
                document.maximum_track_bytes,
            )
            .map_err(map_studio_error)?;
            if !session.can_start_native_encoding(&graph) {
                return Err(NativeDesktopBackendError::Unavailable);
            }
        }
        (JournalBoundary::RecordingGraphPrepared, None) => {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        _ => return Err(NativeDesktopBackendError::Unavailable),
    }
    archive_current_journal(projects_directory, discovered, journal.snapshot())
}

fn take_recovery_ownership(
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
) -> Result<(), NativeDesktopBackendError> {
    journal
        .take_ownership(
            journal.snapshot().revision,
            journal.snapshot().fence,
            random_worker_id()?,
        )
        .map_err(map_studio_error)
}

fn recover_recording(
    projects_root: &Path,
    originals_root: &Path,
    project_identity: [u8; 16],
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    probe_duration: &mut dyn FnMut(TrackKind, &Path) -> Result<RationalTime, StudioError>,
) -> Result<StudioProjectManifest, NativeDesktopBackendError> {
    let start_hint = journal
        .snapshot()
        .pending_asset
        .as_ref()
        .map(|pending| pending.asset.start);
    let mut originals =
        FilesystemStudioOriginalStore::new(originals_root).map_err(map_studio_error)?;
    drive_pending_asset(project_identity, journal, &mut originals)?;
    if journal.snapshot().boundary != JournalBoundary::CaptureStarted {
        return Err(NativeDesktopBackendError::Internal);
    }
    let document = originals
        .find_persisted_recording_graph(journal.snapshot().project_id)
        .map_err(map_studio_error)?
        .ok_or(NativeDesktopBackendError::Filesystem)?;
    let session = FilesystemStudioRecordingSession::recover_reconciling_originals(
        &mut originals,
        document.graph,
        document.maximum_track_bytes,
    )
    .map_err(map_studio_error)?;
    let recovered = session
        .finish_recovered(start_hint, probe_duration)
        .map_err(map_studio_error)?;
    let mut durable_assets = Vec::with_capacity(recovered.len());
    for asset in recovered {
        if asset.commit_state == AssetCommitState::DurableOriginal {
            durable_assets.push(asset);
            continue;
        }
        durable_assets.push(commit_asset(
            project_identity,
            journal,
            &mut originals,
            asset,
        )?);
    }
    let manifest = StudioProjectManifest {
        version: STUDIO_PROJECT_VERSION,
        id: journal.snapshot().project_id,
        revision: 1,
        state: StudioState::Editing,
        assets: durable_assets,
        edits: frame_media::EditSpec::default(),
    };
    let mut projects = FilesystemStudioProjectStore::new(projects_root, journal.snapshot().fence)
        .map_err(map_studio_error)?;
    projects
        .create_project(&manifest)
        .map_err(map_studio_error)?;
    advance_journal(
        journal,
        project_identity,
        JournalBoundary::RecordingStopped,
        ReceiptKind::RecordingStopped,
        None,
    )?;
    Ok(manifest)
}

fn drive_pending_asset(
    project_identity: [u8; 16],
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    originals: &mut FilesystemStudioOriginalStore,
) -> Result<(), NativeDesktopBackendError> {
    loop {
        match journal.snapshot().boundary {
            JournalBoundary::CaptureStarted => return Ok(()),
            JournalBoundary::TempAssetReserved => {
                let asset = pending_asset(journal)?;
                advance_journal(
                    journal,
                    project_identity,
                    JournalBoundary::TempAssetDurable,
                    ReceiptKind::TempDurable,
                    Some(&asset),
                )?;
            }
            JournalBoundary::TempAssetDurable => {
                let asset = pending_asset(journal)?;
                advance_journal(
                    journal,
                    project_identity,
                    JournalBoundary::AssetCommitRequested,
                    ReceiptKind::AssetCommitRequested,
                    Some(&asset),
                )?;
            }
            JournalBoundary::AssetCommitRequested => {
                let pending = journal
                    .snapshot()
                    .pending_asset
                    .clone()
                    .ok_or(NativeDesktopBackendError::Internal)?;
                let durable = commit_verified_temporary(
                    originals,
                    TempAssetCommitTicket::new(
                        journal.snapshot().project_id,
                        pending.operation_id,
                        journal.snapshot().fence,
                        pending.asset,
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
            }
            JournalBoundary::AssetCommitted => {
                advance_journal(
                    journal,
                    project_identity,
                    JournalBoundary::CaptureStarted,
                    ReceiptKind::CaptureStarted,
                    None,
                )?;
            }
            _ => return Err(NativeDesktopBackendError::Unavailable),
        }
    }
}

fn pending_asset(
    journal: &DurableStudioJournal<FilesystemStudioJournalStore>,
) -> Result<StudioAsset, NativeDesktopBackendError> {
    journal
        .snapshot()
        .pending_asset
        .as_ref()
        .map(|pending| pending.asset.clone())
        .ok_or(NativeDesktopBackendError::Internal)
}

fn commit_asset(
    project_identity: [u8; 16],
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    originals: &mut FilesystemStudioOriginalStore,
    asset: StudioAsset,
) -> Result<StudioAsset, NativeDesktopBackendError> {
    advance_journal(
        journal,
        project_identity,
        JournalBoundary::TempAssetReserved,
        ReceiptKind::TempReserved,
        Some(&asset),
    )?;
    advance_journal(
        journal,
        project_identity,
        JournalBoundary::TempAssetDurable,
        ReceiptKind::TempDurable,
        Some(&asset),
    )?;
    let operation = advance_journal(
        journal,
        project_identity,
        JournalBoundary::AssetCommitRequested,
        ReceiptKind::AssetCommitRequested,
        Some(&asset),
    )?;
    let durable = commit_verified_temporary(
        originals,
        TempAssetCommitTicket::new(
            journal.snapshot().project_id,
            operation,
            journal.snapshot().fence,
            asset,
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
    Ok(durable)
}

fn recover_edit_save(
    projects_root: &Path,
    project_identity: [u8; 16],
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
) -> Result<StudioProjectManifest, NativeDesktopBackendError> {
    let pending = journal
        .snapshot()
        .pending_edit
        .clone()
        .ok_or(NativeDesktopBackendError::Internal)?;
    let mut projects = FilesystemStudioProjectStore::new(projects_root, journal.snapshot().fence)
        .map_err(map_studio_error)?;
    let current = projects
        .probe_project(journal.snapshot().project_id)
        .map_err(map_studio_error)?
        .ok_or(NativeDesktopBackendError::Filesystem)?;
    let committed = if current.revision == pending.expected_project_revision {
        commit_edit_save(
            &mut projects,
            EditSaveTicket::new(
                &current,
                pending.operation_id,
                journal.snapshot().fence,
                pending.edits.clone(),
            )
            .map_err(map_studio_error)?,
        )
        .map_err(map_studio_error)?
    } else if current.revision == pending.edits.revision
        && current.edits == pending.edits
        && current
            .assets
            .iter()
            .all(|asset| asset.commit_state == AssetCommitState::DurableOriginal)
    {
        current
    } else {
        return Err(NativeDesktopBackendError::Filesystem);
    };
    advance_edit_journal(journal, project_identity, pending)?;
    Ok(committed)
}

fn advance_edit_journal(
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    project_identity: [u8; 16],
    pending: PendingEditSave,
) -> Result<(), NativeDesktopBackendError> {
    let operation_id = random_operation_id()?;
    let encoded = StudioDocumentCodec::encode_edit(&pending.edits).map_err(map_studio_error)?;
    let expected_project_revision = pending.expected_project_revision;
    let digest = |domain: &[u8]| {
        let mut canonical = Vec::with_capacity(domain.len() + encoded.len() + 64);
        canonical.extend_from_slice(domain);
        canonical.push(0);
        canonical.extend_from_slice(&project_identity);
        canonical.extend_from_slice(&expected_project_revision.to_be_bytes());
        canonical.extend_from_slice(&encoded);
        strong_sha256(&canonical)
    };
    let command_digest = digest(b"frame-desktop-studio-edit-recovery-command-v1");
    let outcome_digest = digest(b"frame-desktop-studio-edit-recovery-outcome-v1");
    journal
        .advance(JournalAdvanceRequest {
            expected_revision: journal.snapshot().revision,
            expected_fence: journal.snapshot().fence,
            operation_id,
            command_digest,
            boundary: JournalBoundary::EditSaveCommitted,
            pending_asset: None,
            pending_edit: Some(pending),
            pending_render: None,
            receipt_kind: ReceiptKind::EditCommitted,
            outcome_digest,
        })
        .map_err(map_studio_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::PathBuf};

    use frame_media::{
        AssetCommitState, ColorSpace, EditOperation, EditSpec, FilesystemStudioRecordingSession,
        FrameMemory, PixelFormat, STUDIO_EDIT_VERSION, STUDIO_JOURNAL_VERSION, StudioOperationId,
        StudioProjectId, StudioWorkerId, TimeBase, VideoFrameSpec,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::macos_native_backend::{
        studio_projects::discover,
        studio_recorder::{
            StudioOptionalTracks, StudioRecordingIdentity, advance_journal, recording_graph,
        },
    };

    const MAXIMUM_TRACK_BYTES: u64 = 4_096;

    struct RecoveryFixture {
        _directory: TempDir,
        projects: PathBuf,
        originals: PathBuf,
        project_id: StudioProjectId,
    }

    fn seconds(value: u64) -> RationalTime {
        RationalTime::new(value, TimeBase::new(1).expect("timebase"))
    }

    fn screen_spec() -> VideoFrameSpec {
        VideoFrameSpec {
            width: 160,
            height: 90,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: FrameMemory::Cpu,
        }
    }

    fn identity(marker: u8) -> StudioRecordingIdentity {
        StudioRecordingIdentity {
            project: [marker; 16],
            clock: [marker + 1; 16],
            screen_asset: [marker + 2; 16],
            microphone_asset: [marker + 3; 16],
            system_audio_asset: [marker + 4; 16],
            camera_asset: [marker + 5; 16],
            cursor_asset: [marker + 6; 16],
        }
    }

    fn create_journal(
        projects: &Path,
        project_id: StudioProjectId,
        owner_marker: u8,
    ) -> DurableStudioJournal<FilesystemStudioJournalStore> {
        DurableStudioJournal::create(
            FilesystemStudioJournalStore::new(projects).expect("journal store"),
            frame_media::StudioJournalSnapshot {
                version: STUDIO_JOURNAL_VERSION,
                project_id,
                revision: 1,
                fence: 1,
                owner: StudioWorkerId::from_csprng([owner_marker; 16]).expect("owner"),
                boundary: JournalBoundary::Created,
                last_operation_id: None,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipts: BTreeMap::new(),
            },
        )
        .expect("journal")
    }

    fn recording_fixture(marker: u8, boundary: JournalBoundary) -> RecoveryFixture {
        assert!(matches!(
            boundary,
            JournalBoundary::CaptureStarted
                | JournalBoundary::TempAssetReserved
                | JournalBoundary::TempAssetDurable
                | JournalBoundary::AssetCommitRequested
                | JournalBoundary::AssetCommitted
        ));
        let directory = tempfile::tempdir().expect("recovery fixture");
        let projects = directory.path().join("projects");
        let originals = directory.path().join("originals");
        fs::create_dir_all(&projects).expect("projects root");
        fs::create_dir_all(&originals).expect("originals root");
        let projects = fs::canonicalize(projects).expect("canonical projects root");
        let originals = fs::canonicalize(originals).expect("canonical originals root");
        let recording_identity = identity(marker);
        let project_identity = recording_identity.project;
        let graph = recording_graph(
            recording_identity,
            screen_spec(),
            30,
            StudioOptionalTracks::default(),
        )
        .expect("recording graph");
        let project_id = graph.project_id;
        let mut journal = create_journal(&projects, project_id, marker + 6);
        let mut original_store =
            FilesystemStudioOriginalStore::new(&originals).expect("original store");
        let mut session =
            FilesystemStudioRecordingSession::begin(&original_store, graph, MAXIMUM_TRACK_BYTES)
                .expect("recording session");
        advance_journal(
            &mut journal,
            project_identity,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
            None,
        )
        .expect("graph prepared");
        advance_journal(
            &mut journal,
            project_identity,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
            None,
        )
        .expect("capture started");
        session
            .write_encoded_chunk(TrackKind::Screen, b"streamable-webm-before-crash")
            .expect("encoded bytes");
        let temporary = session
            .finish(seconds(0), seconds(2))
            .expect("sealed temporary")
            .into_iter()
            .next()
            .expect("screen temporary");

        if boundary != JournalBoundary::CaptureStarted {
            advance_journal(
                &mut journal,
                project_identity,
                JournalBoundary::TempAssetReserved,
                ReceiptKind::TempReserved,
                Some(&temporary),
            )
            .expect("temporary reserved");
        }
        if matches!(
            boundary,
            JournalBoundary::TempAssetDurable
                | JournalBoundary::AssetCommitRequested
                | JournalBoundary::AssetCommitted
        ) {
            advance_journal(
                &mut journal,
                project_identity,
                JournalBoundary::TempAssetDurable,
                ReceiptKind::TempDurable,
                Some(&temporary),
            )
            .expect("temporary durable");
        }
        let mut commit_operation = None;
        if matches!(
            boundary,
            JournalBoundary::AssetCommitRequested | JournalBoundary::AssetCommitted
        ) {
            commit_operation = Some(
                advance_journal(
                    &mut journal,
                    project_identity,
                    JournalBoundary::AssetCommitRequested,
                    ReceiptKind::AssetCommitRequested,
                    Some(&temporary),
                )
                .expect("asset commit requested"),
            );
        }
        if boundary == JournalBoundary::AssetCommitted {
            let durable = commit_verified_temporary(
                &mut original_store,
                TempAssetCommitTicket::new(
                    project_id,
                    commit_operation.expect("commit operation"),
                    journal.snapshot().fence,
                    temporary,
                )
                .expect("commit ticket"),
            )
            .expect("durable original");
            advance_journal(
                &mut journal,
                project_identity,
                JournalBoundary::AssetCommitted,
                ReceiptKind::AssetCommitted,
                Some(&durable),
            )
            .expect("asset committed");
        }
        assert_eq!(journal.snapshot().boundary, boundary);
        drop(journal);

        RecoveryFixture {
            _directory: directory,
            projects,
            originals,
            project_id,
        }
    }

    fn unstarted_fixture(marker: u8, graph_prepared: bool) -> RecoveryFixture {
        let directory = tempfile::tempdir().expect("archive fixture");
        let projects = directory.path().join("projects");
        let originals = directory.path().join("originals");
        fs::create_dir_all(&projects).expect("projects root");
        fs::create_dir_all(&originals).expect("originals root");
        let projects = fs::canonicalize(projects).expect("canonical projects root");
        let originals = fs::canonicalize(originals).expect("canonical originals root");
        let recording_identity = identity(marker);
        let project_identity = recording_identity.project;
        let graph = recording_graph(
            recording_identity,
            screen_spec(),
            30,
            StudioOptionalTracks::default(),
        )
        .expect("recording graph");
        let project_id = graph.project_id;
        let mut journal = create_journal(&projects, project_id, marker + 6);
        if graph_prepared {
            let store = FilesystemStudioOriginalStore::new(&originals).expect("original store");
            let session =
                FilesystemStudioRecordingSession::begin(&store, graph, MAXIMUM_TRACK_BYTES)
                    .expect("empty recording session");
            drop(session);
            advance_journal(
                &mut journal,
                project_identity,
                JournalBoundary::RecordingGraphPrepared,
                ReceiptKind::GraphPrepared,
                None,
            )
            .expect("graph prepared");
        }
        drop(journal);
        RecoveryFixture {
            _directory: directory,
            projects,
            originals,
            project_id,
        }
    }

    fn discover_one(fixture: &RecoveryFixture) -> DiscoveredStudioProject {
        let rooted = RootedDir::bind(&fixture.projects).expect("rooted projects");
        let mut discovered = discover(&rooted).expect("bounded discovery");
        assert_eq!(discovered.len(), 1);
        discovered.pop().expect("one project")
    }

    fn current_journal(
        fixture: &RecoveryFixture,
    ) -> DurableStudioJournal<FilesystemStudioJournalStore> {
        DurableStudioJournal::open(
            FilesystemStudioJournalStore::new(&fixture.projects).expect("journal store"),
            fixture.project_id,
        )
        .expect("current journal")
    }

    #[test]
    fn recording_recovery_finishes_every_persisted_asset_boundary_idempotently() {
        let boundaries = [
            JournalBoundary::CaptureStarted,
            JournalBoundary::TempAssetReserved,
            JournalBoundary::TempAssetDurable,
            JournalBoundary::AssetCommitRequested,
            JournalBoundary::AssetCommitted,
        ];
        for (index, boundary) in boundaries.into_iter().enumerate() {
            let fixture = recording_fixture(20 + index as u8 * 8, boundary);
            let discovered = discover_one(&fixture);
            let mut probes = 0_u8;
            let manifest = recover_project_with_probe(
                &fixture.projects,
                &fixture.originals,
                &discovered,
                &mut |track, _path| {
                    assert_eq!(track, TrackKind::Screen);
                    probes += 1;
                    Ok(seconds(2))
                },
            )
            .expect("recover exact recording boundary");
            assert_eq!(manifest.id, fixture.project_id);
            assert_eq!(manifest.revision, 1);
            assert_eq!(manifest.state, StudioState::Editing);
            assert_eq!(manifest.assets.len(), 1);
            assert_eq!(
                manifest.assets[0].commit_state,
                AssetCommitState::DurableOriginal
            );
            assert_eq!(manifest.assets[0].duration, seconds(2));
            assert_eq!(
                probes,
                u8::from(boundary == JournalBoundary::CaptureStarted),
                "only an uncommitted track needs a media-duration probe"
            );
            assert_eq!(
                current_journal(&fixture).snapshot().boundary,
                JournalBoundary::RecordingStopped
            );
            let ready = discover_one(&fixture);
            assert_eq!(
                ready.recovery_action(),
                NativeStudioRecoveryAction::OpenEditor
            );
        }
    }

    #[test]
    fn empty_attempt_archive_moves_only_the_journal_and_preserves_the_graph() {
        for (index, graph_prepared) in [false, true].into_iter().enumerate() {
            let fixture = unstarted_fixture(70 + index as u8 * 8, graph_prepared);
            let discovered = discover_one(&fixture);
            let projects = RootedDir::bind(&fixture.projects).expect("rooted projects");
            archive_unstarted_project(
                &fixture.projects,
                &fixture.originals,
                &projects,
                &discovered,
            )
            .expect("archive exact empty attempt");
            assert!(
                discover(&projects)
                    .expect("post-archive discovery")
                    .is_empty()
            );
            assert_eq!(
                fs::read_dir(fixture.projects.join("recovery-archive"))
                    .expect("recovery archive")
                    .count(),
                1
            );
            let store =
                FilesystemStudioOriginalStore::new(&fixture.originals).expect("original store");
            assert_eq!(
                store
                    .find_persisted_recording_graph(fixture.project_id)
                    .expect("graph discovery")
                    .is_some(),
                graph_prepared,
                "archive must not delete a prepared graph or its sinks"
            );
        }
    }

    fn prepare_edit_save(
        fixture: &RecoveryFixture,
        persist_project_before_ack: bool,
    ) -> (PendingEditSave, Vec<StudioAsset>) {
        let mut journal = current_journal(fixture);
        let operation_id =
            StudioOperationId::from_csprng([110; 16]).expect("edit operation identity");
        let edits = EditSpec {
            version: STUDIO_EDIT_VERSION,
            revision: 2,
            operations: vec![EditOperation::Split { at: seconds(1) }],
        };
        let pending = PendingEditSave {
            operation_id,
            expected_project_revision: 1,
            edits: edits.clone(),
        };
        journal
            .advance(JournalAdvanceRequest {
                expected_revision: journal.snapshot().revision,
                expected_fence: journal.snapshot().fence,
                operation_id,
                command_digest: strong_sha256(b"prepare edit save"),
                boundary: JournalBoundary::EditSavePrepared,
                pending_asset: None,
                pending_edit: Some(pending.clone()),
                pending_render: None,
                receipt_kind: ReceiptKind::EditPrepared,
                outcome_digest: strong_sha256(b"edit save prepared"),
            })
            .expect("edit-save boundary");
        let mut projects =
            FilesystemStudioProjectStore::new(&fixture.projects, journal.snapshot().fence)
                .expect("project store");
        let current = projects
            .probe_project(fixture.project_id)
            .expect("project probe")
            .expect("recorded project");
        let originals = current.assets.clone();
        if persist_project_before_ack {
            commit_edit_save(
                &mut projects,
                EditSaveTicket::new(&current, operation_id, journal.snapshot().fence, edits)
                    .expect("edit-save ticket"),
            )
            .expect("persist edit before acknowledgement");
        }
        (pending, originals)
    }

    #[test]
    fn edit_save_recovery_handles_precommit_and_lost_ack_without_changing_originals() {
        for (index, persist_project_before_ack) in [false, true].into_iter().enumerate() {
            let fixture = recording_fixture(90 + index as u8 * 8, JournalBoundary::CaptureStarted);
            let discovered = discover_one(&fixture);
            recover_project_with_probe(
                &fixture.projects,
                &fixture.originals,
                &discovered,
                &mut |_track, _path| Ok(seconds(2)),
            )
            .expect("initial recording recovery");
            let (pending, originals) = prepare_edit_save(&fixture, persist_project_before_ack);
            let discovered = discover_one(&fixture);
            assert_eq!(
                discovered.recovery_action(),
                NativeStudioRecoveryAction::ReconcileEditSave
            );
            let committed = recover_project_with_probe(
                &fixture.projects,
                &fixture.originals,
                &discovered,
                &mut |_track, _path| panic!("edit recovery must not probe media"),
            )
            .expect("edit save recovery");
            assert_eq!(committed.revision, 2);
            assert_eq!(committed.edits, pending.edits);
            assert_eq!(committed.assets, originals);
            let journal = current_journal(&fixture);
            assert_eq!(
                journal.snapshot().boundary,
                JournalBoundary::EditSaveCommitted
            );
            assert_eq!(journal.snapshot().pending_edit.as_ref(), Some(&pending));
        }
    }
}
