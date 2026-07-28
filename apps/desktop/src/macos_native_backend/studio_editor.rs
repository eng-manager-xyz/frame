//! Journal-fenced native Studio editor persistence.
//!
//! The active editor owns the only mutable draft. WebView requests carry
//! bounded mutations and an editor revision, never project paths, identities,
//! journal fences, or canonical manifests.

use std::path::Path;

use frame_media::{
    DurableStudioJournal, EditOperation, EditSaveTicket, EditSpec, FilesystemStudioJournalStore,
    FilesystemStudioProjectStore, JournalAdvanceRequest, JournalBoundary, MAX_STUDIO_EDITS,
    PendingEditSave, RationalTime, ReceiptKind, STUDIO_EDIT_VERSION, StudioDocumentCodec,
    StudioError, StudioProjectManifest, StudioProjectStorePort, StudioTimelineCompiler, TimeBase,
    TimelineSource, TrackKind, commit_edit_save, strong_sha256,
};

use super::{
    studio_projects::DiscoveredStudioProject,
    studio_recorder::{map_studio_error, random_operation_id, random_worker_id},
};
use crate::{
    NativeDesktopBackendError, NativeStudioEditApplyOutcome, NativeStudioEditApplyRequest,
    NativeStudioEditMutation, NativeStudioEditSaveRequest,
};

/// Authenticated draft state for exactly one ready Studio project.
pub(super) struct ActiveStudioEditor {
    manifest: StudioProjectManifest,
    operations: Vec<EditOperation>,
    editor_revision: u64,
    dirty: bool,
}

impl std::fmt::Debug for ActiveStudioEditor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActiveStudioEditor")
            .field("project_id", &"<redacted>")
            .field("project_revision", &self.manifest.revision)
            .field("editor_revision", &self.editor_revision)
            .field("operation_count", &self.operations.len())
            .field("dirty", &self.dirty)
            .finish()
    }
}

impl ActiveStudioEditor {
    pub(super) fn new(manifest: StudioProjectManifest) -> Result<Self, NativeDesktopBackendError> {
        manifest.validate().map_err(map_studio_error)?;
        TimelineSource::from_assets(&manifest.assets)
            .and_then(|source| StudioTimelineCompiler::compile(&source, &manifest.edits))
            .map_err(map_editor_error)?;
        Ok(Self {
            operations: manifest.edits.operations.clone(),
            editor_revision: manifest.revision,
            manifest,
            dirty: false,
        })
    }

    pub(super) fn project_id(&self) -> frame_media::StudioProjectId {
        self.manifest.id
    }

    pub(super) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }

    pub(super) fn clean_manifest(
        &self,
        expected_project_revision: u64,
    ) -> Result<StudioProjectManifest, NativeDesktopBackendError> {
        if self.dirty || self.manifest.revision != expected_project_revision {
            return Err(NativeDesktopBackendError::InvalidEdit);
        }
        Ok(self.manifest.clone())
    }

    pub(super) fn apply(
        &mut self,
        request: &NativeStudioEditApplyRequest,
    ) -> Result<NativeStudioEditApplyOutcome, NativeDesktopBackendError> {
        if request.base_editor_revision != self.editor_revision {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        let next_editor_revision = self
            .editor_revision
            .checked_add(1)
            .ok_or(NativeDesktopBackendError::InvalidEdit)?;
        let next_project_revision = self
            .manifest
            .revision
            .checked_add(1)
            .ok_or(NativeDesktopBackendError::InvalidEdit)?;
        let mut operations = self.operations.clone();
        apply_mutation(&mut operations, &self.manifest, &request.mutation)?;
        if operations.len() > MAX_STUDIO_EDITS {
            return Err(NativeDesktopBackendError::InvalidEdit);
        }
        let edits = EditSpec {
            version: STUDIO_EDIT_VERSION,
            revision: next_project_revision,
            operations: operations.clone(),
        };
        let source =
            TimelineSource::from_assets(&self.manifest.assets).map_err(map_editor_error)?;
        StudioTimelineCompiler::compile(&source, &edits).map_err(map_editor_error)?;

        self.operations = operations;
        self.editor_revision = next_editor_revision;
        self.dirty = true;
        Ok(NativeStudioEditApplyOutcome {
            base_editor_revision: request.base_editor_revision,
            editor_revision: next_editor_revision,
        })
    }

    /// Persists the complete draft through a durable journal transaction.
    ///
    /// Any error after ownership is acquired is treated as an uncertain
    /// outcome by the caller, which must discard this active authority and
    /// force recovery/discovery before another edit.
    pub(super) fn save(
        &mut self,
        projects_root: &Path,
        discovered: &DiscoveredStudioProject,
        request: NativeStudioEditSaveRequest,
    ) -> Result<StudioProjectManifest, NativeDesktopBackendError> {
        if request.expected_editor_revision != self.editor_revision || !self.dirty {
            return Err(NativeDesktopBackendError::InvalidEdit);
        }
        if discovered.project_id() != self.manifest.id
            || discovered.manifest() != Some(&self.manifest)
        {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }

        let next_revision = self
            .manifest
            .revision
            .checked_add(1)
            .ok_or(NativeDesktopBackendError::InvalidEdit)?;
        let edits = EditSpec {
            version: STUDIO_EDIT_VERSION,
            revision: next_revision,
            operations: self.operations.clone(),
        };
        let source =
            TimelineSource::from_assets(&self.manifest.assets).map_err(map_editor_error)?;
        StudioTimelineCompiler::compile(&source, &edits).map_err(map_editor_error)?;

        let journal_store =
            FilesystemStudioJournalStore::new(projects_root).map_err(map_persistence_error)?;
        let mut journal = DurableStudioJournal::open(journal_store, self.manifest.id)
            .map_err(map_persistence_error)?;
        if journal.snapshot() != discovered.journal() {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        journal
            .take_ownership(
                journal.snapshot().revision,
                journal.snapshot().fence,
                random_worker_id()?,
            )
            .map_err(map_persistence_error)?;

        let edit_operation = random_operation_id()?;
        let pending = PendingEditSave {
            operation_id: edit_operation,
            expected_project_revision: self.manifest.revision,
            edits: edits.clone(),
        };
        advance_edit_boundary(
            &mut journal,
            discovered.project_identity(),
            JournalBoundary::EditSavePrepared,
            ReceiptKind::EditPrepared,
            pending.clone(),
        )?;

        let mut projects =
            FilesystemStudioProjectStore::new(projects_root, journal.snapshot().fence)
                .map_err(map_persistence_error)?;
        let current = projects
            .probe_project(self.manifest.id)
            .map_err(map_persistence_error)?
            .ok_or(NativeDesktopBackendError::Filesystem)?;
        if current != self.manifest {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        let committed = commit_edit_save(
            &mut projects,
            EditSaveTicket::new(&current, edit_operation, journal.snapshot().fence, edits)
                .map_err(map_editor_error)?,
        )
        .map_err(map_persistence_error)?;
        advance_edit_boundary(
            &mut journal,
            discovered.project_identity(),
            JournalBoundary::EditSaveCommitted,
            ReceiptKind::EditCommitted,
            pending,
        )?;
        if committed.assets != self.manifest.assets
            || committed.revision != next_revision
            || committed.edits.operations != self.operations
        {
            return Err(NativeDesktopBackendError::Filesystem);
        }

        self.manifest = committed.clone();
        self.operations = committed.edits.operations.clone();
        self.dirty = false;
        Ok(committed)
    }
}

fn apply_mutation(
    operations: &mut Vec<EditOperation>,
    manifest: &StudioProjectManifest,
    mutation: &NativeStudioEditMutation,
) -> Result<(), NativeDesktopBackendError> {
    let time = |milliseconds| {
        Ok(RationalTime::new(
            milliseconds,
            TimeBase::new(1_000).map_err(map_editor_error)?,
        ))
    };
    match mutation {
        NativeStudioEditMutation::Trim { start_ms, end_ms } => {
            operations.retain(|operation| !matches!(operation, EditOperation::Trim { .. }));
            operations.push(EditOperation::Trim {
                start: time(*start_ms)?,
                end: time(*end_ms)?,
            });
        }
        NativeStudioEditMutation::DeleteRange { start_ms, end_ms } => {
            operations.push(EditOperation::DeleteRange {
                start: time(*start_ms)?,
                end: time(*end_ms)?,
            });
        }
        NativeStudioEditMutation::Split { at_ms } => {
            operations.push(EditOperation::Split { at: time(*at_ms)? });
        }
        NativeStudioEditMutation::Speed {
            start_ms,
            end_ms,
            rate_milli,
        } => {
            operations.push(EditOperation::Speed {
                start: time(*start_ms)?,
                end: time(*end_ms)?,
                numerator: u32::from(*rate_milli),
                denominator: 1_000,
            });
        }
        NativeStudioEditMutation::AudioGain {
            start_ms,
            end_ms,
            gain_millibels,
        } => {
            let tracks = [TrackKind::Microphone, TrackKind::SystemAudio]
                .into_iter()
                .filter(|track| manifest.assets.iter().any(|asset| asset.track == *track))
                .collect::<Vec<_>>();
            if tracks.is_empty() {
                return Err(NativeDesktopBackendError::InvalidEdit);
            }
            for track in tracks {
                operations.push(EditOperation::AudioGain {
                    track,
                    start: time(*start_ms)?,
                    end: time(*end_ms)?,
                    gain_millibels: *gain_millibels,
                    muted: false,
                });
            }
        }
    }
    Ok(())
}

fn advance_edit_boundary(
    journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
    project_identity: [u8; 16],
    boundary: JournalBoundary,
    receipt_kind: ReceiptKind,
    pending: PendingEditSave,
) -> Result<(), NativeDesktopBackendError> {
    let operation_id = if boundary == JournalBoundary::EditSavePrepared {
        pending.operation_id
    } else {
        random_operation_id()?
    };
    let command_digest = edit_digest(
        b"frame-desktop-studio-edit-command-v1",
        project_identity,
        boundary,
        &pending,
    )?;
    let outcome_digest = edit_digest(
        b"frame-desktop-studio-edit-outcome-v1",
        project_identity,
        boundary,
        &pending,
    )?;
    journal
        .advance(JournalAdvanceRequest {
            expected_revision: journal.snapshot().revision,
            expected_fence: journal.snapshot().fence,
            operation_id,
            command_digest,
            boundary,
            pending_asset: None,
            pending_edit: Some(pending),
            pending_render: None,
            receipt_kind,
            outcome_digest,
        })
        .map_err(map_persistence_error)?;
    Ok(())
}

fn edit_digest(
    domain: &[u8],
    project_identity: [u8; 16],
    boundary: JournalBoundary,
    pending: &PendingEditSave,
) -> Result<frame_media::Sha256Digest, NativeDesktopBackendError> {
    let encoded = StudioDocumentCodec::encode_edit(&pending.edits).map_err(map_studio_error)?;
    let mut canonical = Vec::with_capacity(domain.len() + encoded.len() + 64);
    canonical.extend_from_slice(domain);
    canonical.push(0);
    canonical.extend_from_slice(&project_identity);
    canonical.push(boundary.canonical_tag());
    canonical.extend_from_slice(&pending.expected_project_revision.to_be_bytes());
    canonical.extend_from_slice(&encoded);
    Ok(strong_sha256(&canonical))
}

fn map_editor_error(error: StudioError) -> NativeDesktopBackendError {
    match error {
        StudioError::StorageIo | StudioError::UnsafeStoragePath => {
            NativeDesktopBackendError::Filesystem
        }
        StudioError::StaleJournal => NativeDesktopBackendError::StaleCatalog,
        _ => NativeDesktopBackendError::InvalidEdit,
    }
}

fn map_persistence_error(error: StudioError) -> NativeDesktopBackendError {
    match error {
        StudioError::StorageIo
        | StudioError::UnsafeStoragePath
        | StudioError::AmbiguousJournalCommit
        | StudioError::AmbiguousEditSave => NativeDesktopBackendError::Filesystem,
        StudioError::StaleJournal | StudioError::EditSaveMismatch => {
            NativeDesktopBackendError::StaleCatalog
        }
        _ => NativeDesktopBackendError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use frame_media::{
        AssetChecksum, AssetCommitState, FrameRate, JournalAdvanceRequest, STUDIO_ASSET_VERSION,
        STUDIO_JOURNAL_VERSION, STUDIO_PROJECT_VERSION, StudioAsset, StudioAssetEncoding,
        StudioAssetId, StudioJournalSnapshot, StudioOperationId, StudioProjectId, StudioSourceName,
        StudioState, StudioVideoRawCaps, StudioWorkerId,
    };

    use super::*;
    use crate::{
        NativeStudioEditApplyRequest, NativeStudioEditMutation,
        macos_native_backend::studio_projects::discover, rooted_io::RootedDir,
    };

    fn seconds(value: u64) -> RationalTime {
        RationalTime::new(value, TimeBase::new(1).expect("timebase"))
    }

    fn screen_asset() -> StudioAsset {
        StudioAsset {
            version: STUDIO_ASSET_VERSION,
            id: StudioAssetId::from_csprng([2; 16]).expect("asset ID"),
            track: TrackKind::Screen,
            source_name: StudioSourceName::new("screen.webm").expect("source name"),
            byte_len: 4_096,
            start: seconds(0),
            duration: seconds(10),
            checksum: AssetChecksum::from_bytes([3; 32]).expect("checksum"),
            commit_state: AssetCommitState::DurableOriginal,
            encoding: StudioAssetEncoding::recording_vp8_webm(StudioVideoRawCaps {
                width: 1_280,
                height: 720,
                frame_rate: FrameRate {
                    numerator: 30,
                    denominator: 1,
                },
                pixel_format: frame_media::PixelFormat::Bgra8,
            })
            .expect("encoding"),
        }
    }

    fn ready_manifest(project_id: StudioProjectId) -> StudioProjectManifest {
        StudioProjectManifest {
            version: STUDIO_PROJECT_VERSION,
            id: project_id,
            revision: 1,
            state: StudioState::Editing,
            assets: vec![screen_asset()],
            edits: EditSpec::default(),
        }
    }

    fn advance(
        journal: &mut DurableStudioJournal<FilesystemStudioJournalStore>,
        marker: u8,
        boundary: JournalBoundary,
        receipt_kind: ReceiptKind,
    ) {
        let operation_id = StudioOperationId::from_csprng([marker; 16]).expect("operation ID");
        journal
            .advance(JournalAdvanceRequest {
                expected_revision: journal.snapshot().revision,
                expected_fence: journal.snapshot().fence,
                operation_id,
                command_digest: strong_sha256(&[marker, 1]),
                boundary,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipt_kind,
                outcome_digest: strong_sha256(&[marker, 2]),
            })
            .expect("journal advance");
    }

    #[test]
    fn draft_revision_advances_only_after_canonical_compilation() {
        let manifest =
            ready_manifest(StudioProjectId::from_csprng([1; 16]).expect("project identity"));
        let mut editor = ActiveStudioEditor::new(manifest).expect("editor");
        let invalid = NativeStudioEditApplyRequest {
            base_editor_revision: 1,
            mutation: NativeStudioEditMutation::Trim {
                start_ms: 9_000,
                end_ms: 9_000,
            },
        };
        assert_eq!(
            editor.apply(&invalid),
            Err(NativeDesktopBackendError::InvalidEdit)
        );
        assert_eq!(editor.editor_revision(), 1);

        let valid = NativeStudioEditApplyRequest {
            base_editor_revision: 1,
            mutation: NativeStudioEditMutation::Trim {
                start_ms: 1_000,
                end_ms: 9_000,
            },
        };
        assert_eq!(editor.apply(&valid).expect("valid edit").editor_revision, 2);
        assert_eq!(editor.editor_revision(), 2);
    }

    #[test]
    fn save_is_journal_fenced_and_preserves_immutable_originals() {
        let directory = tempfile::tempdir().expect("fixture");
        let projects = directory.path().join("projects");
        fs::create_dir(&projects).expect("projects root");
        let projects = fs::canonicalize(projects).expect("canonical projects root");
        let project_id = StudioProjectId::from_csprng([11; 16]).expect("project identity");
        let mut journal = DurableStudioJournal::create(
            FilesystemStudioJournalStore::new(&projects).expect("journal store"),
            StudioJournalSnapshot {
                version: STUDIO_JOURNAL_VERSION,
                project_id,
                revision: 1,
                fence: 1,
                owner: StudioWorkerId::from_csprng([12; 16]).expect("owner"),
                boundary: JournalBoundary::Created,
                last_operation_id: None,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipts: BTreeMap::new(),
            },
        )
        .expect("journal");
        advance(
            &mut journal,
            13,
            JournalBoundary::RecordingGraphPrepared,
            ReceiptKind::GraphPrepared,
        );
        advance(
            &mut journal,
            14,
            JournalBoundary::CaptureStarted,
            ReceiptKind::CaptureStarted,
        );
        advance(
            &mut journal,
            15,
            JournalBoundary::RecordingStopped,
            ReceiptKind::RecordingStopped,
        );
        let manifest = ready_manifest(project_id);
        let originals = manifest.assets.clone();
        let mut project_store =
            FilesystemStudioProjectStore::new(&projects, journal.snapshot().fence)
                .expect("project store");
        project_store
            .create_project(&manifest)
            .expect("ready project");
        drop(journal);

        let rooted = RootedDir::bind(&projects).expect("rooted projects");
        let discovered = discover(&rooted)
            .expect("discovery")
            .pop()
            .expect("one project");
        let mut editor = ActiveStudioEditor::new(manifest).expect("editor");
        editor
            .apply(&NativeStudioEditApplyRequest {
                base_editor_revision: 1,
                mutation: NativeStudioEditMutation::Split { at_ms: 5_000 },
            })
            .expect("draft edit");
        let committed = editor
            .save(
                &projects,
                &discovered,
                NativeStudioEditSaveRequest {
                    expected_editor_revision: 2,
                },
            )
            .expect("durable save");
        assert_eq!(committed.revision, 2);
        assert_eq!(committed.assets, originals);
        assert_eq!(
            committed.edits.operations,
            vec![EditOperation::Split {
                at: RationalTime::new(5_000, TimeBase::new(1_000).expect("timebase"))
            }]
        );

        let ready = discover(&rooted)
            .expect("post-save discovery")
            .pop()
            .expect("saved project");
        assert_eq!(ready.manifest(), Some(&committed));
        assert_eq!(ready.journal().boundary, JournalBoundary::EditSaveCommitted);
    }
}
