use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use frame_media::{
    AssetCommitState, DurableStudioJournal, FilesystemLegacyCapProjectPort,
    FilesystemStudioJournalStore, FilesystemStudioOriginalStore, FilesystemStudioProjectStore,
    LegacyCapProjectPort, LegacyIdAssignment, LegacyImportOutcome, MAX_STUDIO_DOCUMENT_BYTES,
    ReceiptKind, STUDIO_JOURNAL_VERSION, Sha256Digest, StudioAssetId, StudioDocumentCodec,
    StudioJournalSnapshot, StudioOperationId, StudioOperationReceipt, StudioProjectId,
    StudioWorkerId, TempAssetCommitTicket, commit_legacy_import_journal, commit_verified_temporary,
    import_legacy_cap, legacy_import_command_digest, legacy_import_outcome_digest,
};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;

use crate::{
    LEGACY_PROJECT_CATALOG_VERSION, LegacyImportError, LegacyImportReceipt, LegacyProjectCatalog,
    LegacyProjectCatalogAvailability, LegacyProjectStatus, LegacyProjectSummary,
    LegacySettingsInspection, MAX_LEGACY_PROJECT_CATALOG_ENTRIES,
};

const MAX_RECORDING_ROOTS: usize = 22;
const MAX_ROOT_DIRECTORY_ENTRIES: usize = 2_048;
const MAX_STORE_BYTES: u64 = 1024 * 1024;
const INITIAL_JOURNAL_REVISION: u64 = 1;
const INITIAL_JOURNAL_FENCE: u64 = 1;

#[derive(Clone)]
struct Candidate {
    source_root: PathBuf,
    source_digest: Option<Sha256Digest>,
    status: LegacyProjectStatus,
}

impl std::fmt::Debug for Candidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Candidate")
            .field("source_root", &"<redacted>")
            .field("source_digest", &self.source_digest)
            .field("status", &self.status)
            .finish()
    }
}

pub struct LegacyProjectMigrationService {
    cap_app_data_root: PathBuf,
    frame_projects_root: PathBuf,
    frame_originals_root: PathBuf,
    generation: u64,
    candidates: BTreeMap<String, Candidate>,
}

impl std::fmt::Debug for LegacyProjectMigrationService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyProjectMigrationService")
            .field("cap_app_data_root", &"<redacted>")
            .field("frame_projects_root", &"<redacted>")
            .field("frame_originals_root", &"<redacted>")
            .field("generation", &self.generation)
            .field("candidate_count", &self.candidates.len())
            .finish()
    }
}

impl LegacyProjectMigrationService {
    pub fn new(
        cap_app_data_root: impl Into<PathBuf>,
        frame_projects_root: impl Into<PathBuf>,
        frame_originals_root: impl Into<PathBuf>,
    ) -> Result<Self, LegacyImportError> {
        let cap_app_data_root = cap_app_data_root.into();
        let frame_projects_root = prepare_frame_root(&frame_projects_root.into())?;
        let frame_originals_root = prepare_frame_root(&frame_originals_root.into())?;
        if frame_projects_root == frame_originals_root {
            return Err(LegacyImportError::Storage);
        }
        Ok(Self {
            cap_app_data_root,
            frame_projects_root,
            frame_originals_root,
            generation: 0,
            candidates: BTreeMap::new(),
        })
    }

    pub fn scan(&mut self) -> Result<LegacyProjectCatalog, LegacyImportError> {
        let (roots, settings_inspection) = self.recording_roots()?;
        let mut project_roots = BTreeSet::new();
        for root in roots {
            let Ok(entries) = read_directory_bounded(&root, MAX_ROOT_DIRECTORY_ENTRIES) else {
                continue;
            };
            for entry in entries {
                let path = entry.path();
                let is_cap = path.extension().and_then(|value| value.to_str()) == Some("cap");
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !is_cap || !file_type.is_dir() || file_type.is_symlink() {
                    continue;
                }
                let Ok(canonical) = path.canonicalize() else {
                    continue;
                };
                if canonical.parent() == Some(root.as_path()) {
                    project_roots.insert(canonical);
                }
            }
        }
        if project_roots.len() > MAX_LEGACY_PROJECT_CATALOG_ENTRIES {
            return Err(LegacyImportError::Bound);
        }

        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or(LegacyImportError::Bound)?;
        let mut candidates = BTreeMap::new();
        let mut projects = Vec::with_capacity(project_roots.len());
        for (index, source_root) in project_roots.into_iter().enumerate() {
            let (
                mut status,
                source_digest,
                source_asset_count,
                supported_effect_count,
                unsupported,
            ) = inspect_candidate(&source_root)?;
            if status == LegacyProjectStatus::Importable
                && let Some(existing) =
                    source_digest.and_then(|digest| self.existing_import_status(digest))
            {
                status = existing;
            }
            let token = fresh_unique_token(&candidates, "legacy-project")?;
            let ordinal = u16::try_from(index + 1).map_err(|_| LegacyImportError::Bound)?;
            projects.push(LegacyProjectSummary {
                project_token: token.clone(),
                ordinal,
                status,
                source_asset_count,
                supported_effect_count,
                unsupported_effect_count: unsupported,
            });
            candidates.insert(
                token,
                Candidate {
                    source_root,
                    source_digest,
                    status,
                },
            );
        }
        let catalog = LegacyProjectCatalog {
            schema_version: LEGACY_PROJECT_CATALOG_VERSION,
            generation: next_generation,
            availability: LegacyProjectCatalogAvailability::Ready,
            settings_inspection,
            projects,
        };
        catalog.validate()?;
        self.generation = next_generation;
        self.candidates = candidates;
        Ok(catalog)
    }

    pub fn import(
        &mut self,
        catalog_generation: u64,
        project_token: &str,
    ) -> Result<LegacyImportReceipt, LegacyImportError> {
        if catalog_generation == 0 || catalog_generation != self.generation {
            return Err(LegacyImportError::StaleCatalog);
        }
        let candidate = self
            .candidates
            .get(project_token)
            .cloned()
            .ok_or(LegacyImportError::StaleCatalog)?;
        match candidate.status {
            LegacyProjectStatus::Importable => {}
            LegacyProjectStatus::Imported => return Err(LegacyImportError::StaleCatalog),
            LegacyProjectStatus::NeedsReview => return Err(LegacyImportError::NeedsReview),
            LegacyProjectStatus::Unsupported => return Err(LegacyImportError::Unsupported),
            LegacyProjectStatus::Invalid => return Err(LegacyImportError::InvalidProject),
        }

        let mut source = FilesystemLegacyCapProjectPort::open(&candidate.source_root)
            .map_err(map_media_error)?;
        let snapshot = source.read_snapshot().map_err(map_media_error)?;
        let source_asset_count = snapshot_asset_count(&snapshot)?;
        let assignment = LegacyIdAssignment {
            project_id: random_project_id()?,
            asset_ids: (0..source_asset_count)
                .map(|_| random_asset_id())
                .collect::<Result<Vec<_>, _>>()?,
        };
        let LegacyImportOutcome::Imported(import) =
            import_legacy_cap(&mut source, &assignment).map_err(map_media_error)?
        else {
            return Err(LegacyImportError::SourceChanged);
        };
        if candidate.source_digest != Some(import.report.source_digest)
            || import.source_digest_before != import.source_digest_after
        {
            return Err(LegacyImportError::SourceChanged);
        }
        let manifest_digest = frame_media::strong_sha256(
            &StudioDocumentCodec::encode_project(&import.manifest).map_err(map_media_error)?,
        );

        let owner = random_worker_id()?;
        let prepared_operation = random_operation_id()?;
        let prepared_receipt = StudioOperationReceipt {
            operation_id: prepared_operation,
            kind: ReceiptKind::LegacyImportPrepared,
            command_digest: legacy_import_command_digest(import.source_digest_after),
            outcome_digest: legacy_import_outcome_digest(manifest_digest),
        };
        let journal_store = FilesystemStudioJournalStore::new(&self.frame_projects_root)
            .map_err(map_media_error)?;
        let mut journal = DurableStudioJournal::create(
            journal_store,
            StudioJournalSnapshot {
                version: STUDIO_JOURNAL_VERSION,
                project_id: assignment.project_id,
                revision: INITIAL_JOURNAL_REVISION,
                fence: INITIAL_JOURNAL_FENCE,
                owner,
                boundary: frame_media::JournalBoundary::Created,
                last_operation_id: None,
                pending_asset: None,
                pending_edit: None,
                pending_render: None,
                receipts: BTreeMap::from([(prepared_operation, prepared_receipt)]),
            },
        )
        .map_err(map_media_error)?;

        let mut originals = FilesystemStudioOriginalStore::new(&self.frame_originals_root)
            .map_err(map_media_error)?;
        for (entry, durable) in import.copy_plan.entries.iter().zip(&import.manifest.assets) {
            let mut temporary = durable.clone();
            temporary.commit_state = AssetCommitState::Temporary;
            originals
                .stage_legacy_copy(
                    &candidate.source_root,
                    assignment.project_id,
                    entry,
                    &temporary,
                )
                .map_err(map_media_error)?;
            commit_verified_temporary(
                &mut originals,
                TempAssetCommitTicket::new(
                    assignment.project_id,
                    random_operation_id()?,
                    journal.snapshot().fence,
                    temporary,
                )
                .map_err(map_media_error)?,
            )
            .map_err(map_media_error)?;
        }
        let after_copy = source.source_tree_digest().map_err(map_media_error)?;
        if after_copy != import.source_digest_after {
            return Err(LegacyImportError::SourceChanged);
        }

        let mut projects =
            FilesystemStudioProjectStore::new(&self.frame_projects_root, journal.snapshot().fence)
                .map_err(map_media_error)?;
        projects
            .create_project(&import.manifest)
            .map_err(map_media_error)?;
        commit_legacy_import_journal(
            &mut journal,
            random_operation_id()?,
            import.source_digest_after,
            manifest_digest,
        )
        .map_err(map_media_error)?;

        self.candidates.remove(project_token);
        Ok(LegacyImportReceipt {
            imported_assets: u16::try_from(import.manifest.assets.len())
                .map_err(|_| LegacyImportError::Bound)?,
            project_revision: import.manifest.revision,
        })
    }

    fn recording_roots(
        &self,
    ) -> Result<(Vec<PathBuf>, LegacySettingsInspection), LegacyImportError> {
        let default = self.cap_app_data_root.join("recordings");
        let (settings, inspection) = read_cap_settings(&self.cap_app_data_root);
        let mut requested = Vec::with_capacity(MAX_RECORDING_ROOTS);
        requested.push(default);
        if let Some(settings) = settings {
            if let Some(path) = settings.recordings_path {
                requested.push(PathBuf::from(path));
            }
            requested.extend(
                settings
                    .previous_recordings_paths
                    .into_iter()
                    .take(MAX_RECORDING_ROOTS.saturating_sub(requested.len()))
                    .map(PathBuf::from),
            );
        }
        let mut seen = BTreeSet::new();
        let roots = requested
            .into_iter()
            .filter(|path| path.is_absolute() && path.is_dir())
            .filter_map(|path| path.canonicalize().ok())
            .filter(|path| seen.insert(path.clone()))
            .take(MAX_RECORDING_ROOTS)
            .collect();
        Ok((roots, inspection))
    }

    fn existing_import_status(&self, source_digest: Sha256Digest) -> Option<LegacyProjectStatus> {
        let expected = legacy_import_command_digest(source_digest);
        let Ok(entries) =
            read_directory_bounded(&self.frame_projects_root, MAX_ROOT_DIRECTORY_ENTRIES)
        else {
            return None;
        };
        entries.into_iter().find_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("studio-journal") {
                return None;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                return None;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() == 0
                || metadata.len() > MAX_STUDIO_DOCUMENT_BYTES as u64
            {
                return None;
            }
            let Ok(bytes) = fs::read(path) else {
                return None;
            };
            if bytes.len() as u64 != metadata.len() {
                return None;
            }
            let journal = StudioDocumentCodec::decode_journal(&bytes).ok()?;
            let matches_source = journal.receipts.values().any(|receipt| {
                matches!(
                    receipt.kind,
                    ReceiptKind::LegacyImportPrepared | ReceiptKind::LegacyImported
                ) && receipt.command_digest == expected
            });
            matches_source.then_some(
                if journal.boundary == frame_media::JournalBoundary::RecordingStopped {
                    LegacyProjectStatus::Imported
                } else {
                    LegacyProjectStatus::NeedsReview
                },
            )
        })
    }
}

#[derive(Deserialize)]
struct CapStoreDocument {
    #[serde(default)]
    general_settings: Option<CapRecordingSettings>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapRecordingSettings {
    #[serde(default)]
    recordings_path: Option<String>,
    #[serde(default)]
    previous_recordings_paths: Vec<String>,
}

fn read_cap_settings(root: &Path) -> (Option<CapRecordingSettings>, LegacySettingsInspection) {
    for name in ["store", "store.json"] {
        let path = root.join(name);
        if !path.exists() {
            continue;
        }
        let result = (|| {
            let metadata = fs::symlink_metadata(&path).map_err(|_| ())?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() == 0
                || metadata.len() > MAX_STORE_BYTES
            {
                return Err(());
            }
            let mut file = File::open(&path).map_err(|_| ())?;
            let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).map_err(|_| ())?);
            file.read_to_end(&mut bytes).map_err(|_| ())?;
            if bytes.len() as u64 != metadata.len() {
                return Err(());
            }
            serde_json::from_slice::<CapStoreDocument>(&bytes)
                .map(|document| document.general_settings)
                .map_err(|_| ())
        })();
        return match result {
            Ok(settings) => (settings, LegacySettingsInspection::Read),
            Err(()) => (None, LegacySettingsInspection::Invalid),
        };
    }
    (None, LegacySettingsInspection::NotFound)
}

fn inspect_candidate(
    source_root: &Path,
) -> Result<(LegacyProjectStatus, Option<Sha256Digest>, u16, u16, u16), LegacyImportError> {
    let mut source = FilesystemLegacyCapProjectPort::open(source_root).map_err(map_media_error)?;
    let snapshot = match source.read_snapshot() {
        Ok(snapshot) => snapshot,
        Err(_) => return Ok((LegacyProjectStatus::Invalid, None, 0, 0, 0)),
    };
    let asset_count = snapshot_asset_count(&snapshot)?;
    let assignment = LegacyIdAssignment {
        project_id: random_project_id()?,
        asset_ids: (0..asset_count)
            .map(|_| random_asset_id())
            .collect::<Result<Vec<_>, _>>()?,
    };
    let outcome = import_legacy_cap(&mut source, &assignment).map_err(map_media_error)?;
    let (status, report) = match outcome {
        LegacyImportOutcome::Imported(import) => {
            (LegacyProjectStatus::Importable, import.report.clone())
        }
        LegacyImportOutcome::NeedsUserAction(report) => (LegacyProjectStatus::NeedsReview, report),
        LegacyImportOutcome::UnsupportedNewer(report) => (LegacyProjectStatus::Unsupported, report),
    };
    Ok((
        status,
        Some(report.source_digest),
        u16::try_from(report.source_asset_count).map_err(|_| LegacyImportError::Bound)?,
        u16::try_from(report.supported_effect_count).map_err(|_| LegacyImportError::Bound)?,
        u16::try_from(report.unsupported_effects.len()).map_err(|_| LegacyImportError::Bound)?,
    ))
}

fn snapshot_asset_count(
    snapshot: &frame_media::LegacyCapProjectSnapshot,
) -> Result<usize, LegacyImportError> {
    snapshot
        .segments
        .iter()
        .try_fold(0_usize, |count, segment| {
            count
                .checked_add(1)
                .and_then(|value| value.checked_add(usize::from(segment.camera.is_some())))
                .and_then(|value| value.checked_add(usize::from(segment.microphone.is_some())))
                .and_then(|value| value.checked_add(usize::from(segment.system_audio.is_some())))
                .ok_or(LegacyImportError::Bound)
        })
}

fn prepare_frame_root(path: &Path) -> Result<PathBuf, LegacyImportError> {
    fs::create_dir_all(path).map_err(|_| LegacyImportError::Storage)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| LegacyImportError::Storage)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LegacyImportError::Storage);
    }
    path.canonicalize().map_err(|_| LegacyImportError::Storage)
}

fn read_directory_bounded(
    root: &Path,
    maximum: usize,
) -> Result<Vec<fs::DirEntry>, LegacyImportError> {
    let metadata = fs::symlink_metadata(root).map_err(|_| LegacyImportError::Storage)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LegacyImportError::Storage);
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| LegacyImportError::Storage)? {
        if entries.len() == maximum {
            return Err(LegacyImportError::Bound);
        }
        entries.push(entry.map_err(|_| LegacyImportError::Storage)?);
    }
    Ok(entries)
}

fn fresh_unique_token(
    existing: &BTreeMap<String, Candidate>,
    domain: &str,
) -> Result<String, LegacyImportError> {
    (0..32)
        .find_map(|_| {
            let bytes = random_bytes().ok()?;
            let token = format!("{domain}:{}", hex(&bytes));
            (!existing.contains_key(&token)).then_some(token)
        })
        .ok_or(LegacyImportError::RandomUnavailable)
}

fn random_project_id() -> Result<StudioProjectId, LegacyImportError> {
    StudioProjectId::from_csprng(random_bytes()?).map_err(map_media_error)
}

fn random_asset_id() -> Result<StudioAssetId, LegacyImportError> {
    StudioAssetId::from_csprng(random_bytes()?).map_err(map_media_error)
}

fn random_operation_id() -> Result<StudioOperationId, LegacyImportError> {
    StudioOperationId::from_csprng(random_bytes()?).map_err(map_media_error)
}

fn random_worker_id() -> Result<StudioWorkerId, LegacyImportError> {
    StudioWorkerId::from_csprng(random_bytes()?).map_err(map_media_error)
}

fn random_bytes() -> Result<[u8; 16], LegacyImportError> {
    let mut bytes = [0_u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| LegacyImportError::RandomUnavailable)?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(char::from(TABLE[usize::from(byte >> 4)]));
        value.push(char::from(TABLE[usize::from(byte & 0x0f)]));
    }
    value
}

fn map_media_error(error: frame_media::StudioError) -> LegacyImportError {
    match error {
        frame_media::StudioError::LegacySourceChanged => LegacyImportError::SourceChanged,
        frame_media::StudioError::MalformedLegacyProject
        | frame_media::StudioError::LegacyIdAssignmentMismatch => LegacyImportError::InvalidProject,
        frame_media::StudioError::DocumentTooLarge => LegacyImportError::Bound,
        _ => LegacyImportError::Storage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy_fixture_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).expect("create fixture directory");
        for entry in fs::read_dir(source).expect("read fixture directory") {
            let entry = entry.expect("fixture entry");
            let target = destination.join(entry.file_name());
            if entry.file_type().expect("fixture type").is_dir() {
                copy_fixture_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("copy fixture file");
            }
        }
    }

    #[test]
    fn scan_and_import_preserve_cap_source_and_commit_a_reopenable_project() {
        let workspace = tempfile::tempdir().expect("workspace");
        let cap_app_data = workspace.path().join("cap");
        let recordings = cap_app_data.join("recordings");
        let project = recordings.join("example.cap");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/studio/cap-schema-supported");
        copy_fixture_tree(&fixture, &project);
        let mut source_before =
            FilesystemLegacyCapProjectPort::open(&project).expect("open before import");
        let source_before = source_before
            .source_tree_digest()
            .expect("fingerprint before import");
        let projects = workspace.path().join("frame-projects");
        let originals = workspace.path().join("frame-originals");
        let mut service = LegacyProjectMigrationService::new(&cap_app_data, &projects, &originals)
            .expect("service");

        let catalog = service.scan().expect("scan");
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.projects[0].status, LegacyProjectStatus::Importable);
        let token = catalog.projects[0].project_token.clone();
        let receipt = service.import(catalog.generation, &token).expect("import");
        assert_eq!(receipt.imported_assets, 5);
        let mut source_after =
            FilesystemLegacyCapProjectPort::open(&project).expect("open after import");
        assert_eq!(
            source_after
                .source_tree_digest()
                .expect("fingerprint after import"),
            source_before
        );
        assert!(project.join("recording-meta.json").is_file());

        let journal_files = fs::read_dir(&projects)
            .expect("projects")
            .flatten()
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("studio-journal")
            })
            .count();
        let project_files = fs::read_dir(&projects)
            .expect("projects")
            .flatten()
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("studio-project")
            })
            .count();
        assert_eq!((journal_files, project_files), (1, 1));
        drop(service);
        let mut restarted =
            LegacyProjectMigrationService::new(&cap_app_data, &projects, &originals)
                .expect("restart service");
        let refreshed = restarted.scan().expect("refresh after restart");
        assert_eq!(refreshed.projects[0].status, LegacyProjectStatus::Imported);
    }

    #[test]
    fn stale_or_review_required_tokens_never_copy() {
        let workspace = tempfile::tempdir().expect("workspace");
        let cap_app_data = workspace.path().join("cap");
        let recordings = cap_app_data.join("recordings");
        let project = recordings.join("review.cap");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/studio/cap-schema-supported");
        copy_fixture_tree(&fixture, &project);
        let config_path = project.join("project-config.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&fs::read(&config_path).expect("read config"))
                .expect("decode config");
        config["timeline"]["zoomSegments"] = serde_json::json!([{"start": 0, "end": 1}]);
        fs::write(
            &config_path,
            serde_json::to_vec(&config).expect("encode config"),
        )
        .expect("write config");
        let frame_projects = workspace.path().join("frame-projects");
        let frame_originals = workspace.path().join("frame-originals");
        let mut service =
            LegacyProjectMigrationService::new(&cap_app_data, &frame_projects, &frame_originals)
                .expect("service");
        let catalog = service.scan().expect("scan");
        assert_eq!(catalog.projects[0].status, LegacyProjectStatus::NeedsReview);
        assert_eq!(
            service.import(catalog.generation, &catalog.projects[0].project_token),
            Err(LegacyImportError::NeedsReview)
        );
        assert_eq!(
            service.import(
                catalog.generation.saturating_add(1),
                &catalog.projects[0].project_token
            ),
            Err(LegacyImportError::StaleCatalog)
        );
        assert_eq!(
            fs::read_dir(&frame_projects)
                .expect("empty projects")
                .count(),
            0
        );
    }
}
