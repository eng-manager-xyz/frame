//! Descriptor-rooted discovery for durable Studio projects.
//!
//! Directory names and documents are treated as untrusted even though Frame
//! creates the roots with private permissions. Discovery is bounded, opens
//! every candidate through the pinned directory descriptor, and pairs project
//! documents with their exact durable journal identity before minting any
//! WebView-safe handle.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
};

use frame_media::{
    EditSpec, ExactDuration, JournalBoundary, MAX_STUDIO_DOCUMENT_BYTES, StudioDocumentCodec,
    StudioJournalSnapshot, StudioProjectId, StudioProjectManifest, StudioRecoveryDirective,
    StudioState, TrackKind, recovery_directive,
};

use crate::{
    NativeDesktopBackendError, NativeStudioProjectStatus, NativeStudioRecoveryAction,
    rooted_io::{RootedDir, RootedFile, RootedIoError},
};

const MAX_PROJECT_DIRECTORY_ENTRIES: usize = 2_048;
const MAX_DISCOVERED_PROJECTS: usize = crate::MAX_STUDIO_PROJECT_CATALOG_ENTRIES;
const JOURNAL_SUFFIX: &str = ".studio-journal";
const PROJECT_SUFFIX: &str = ".studio-project";
const RECOVERY_ARCHIVE_DIRECTORY: &str = "recovery-archive";

#[derive(Clone)]
pub(super) struct DiscoveredStudioProject {
    project_identity: [u8; 16],
    journal: StudioJournalSnapshot,
    manifest: Option<StudioProjectManifest>,
    journal_relative: PathBuf,
    project_relative: Option<PathBuf>,
    directive: StudioRecoveryDirective,
}

impl std::fmt::Debug for DiscoveredStudioProject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiscoveredStudioProject")
            .field("project_id", &"<redacted>")
            .field("journal_boundary", &self.journal.boundary)
            .field("project_revision", &self.revision())
            .field(
                "asset_count",
                &self.manifest.as_ref().map(|project| project.assets.len()),
            )
            .field("artifact_authority", &"<redacted>")
            .field("directive", &self.directive)
            .finish()
    }
}

impl DiscoveredStudioProject {
    pub(super) const fn project_id(&self) -> StudioProjectId {
        self.journal.project_id
    }

    pub(super) const fn project_identity(&self) -> [u8; 16] {
        self.project_identity
    }

    pub(super) const fn journal(&self) -> &StudioJournalSnapshot {
        &self.journal
    }

    pub(super) fn manifest(&self) -> Option<&StudioProjectManifest> {
        self.manifest.as_ref()
    }

    pub(super) fn status(&self) -> NativeStudioProjectStatus {
        match self.directive {
            StudioRecoveryDirective::OpenEditor
                if self.manifest.as_ref().is_some_and(|manifest| {
                    manifest_matches_open_boundary(&self.journal, manifest)
                }) =>
            {
                NativeStudioProjectStatus::Ready
            }
            StudioRecoveryDirective::OpenEditor
            | StudioRecoveryDirective::RequireOperatorDecision => {
                NativeStudioProjectStatus::AttentionRequired
            }
            _ => NativeStudioProjectStatus::RecoveryRequired,
        }
    }

    pub(super) fn revision(&self) -> Option<u64> {
        self.manifest.as_ref().map(|project| project.revision)
    }

    pub(super) fn asset_count(&self) -> Result<u16, NativeDesktopBackendError> {
        self.manifest.as_ref().map_or(Ok(0), |project| {
            u16::try_from(project.assets.len()).map_err(|_| NativeDesktopBackendError::Filesystem)
        })
    }

    pub(super) fn recovery_action(&self) -> NativeStudioRecoveryAction {
        if self.status() == NativeStudioProjectStatus::AttentionRequired {
            return NativeStudioRecoveryAction::RequiresOperatorDecision;
        }
        match self.directive {
            StudioRecoveryDirective::DiscardUnstartedTemporaryFiles => {
                NativeStudioRecoveryAction::ArchiveUnstartedAttempt
            }
            StudioRecoveryDirective::ResumeOrSealIsolatedTracks
            | StudioRecoveryDirective::DeleteUncommittedTemporaryAsset
            | StudioRecoveryDirective::ProbeAndCommitExactTemporaryAsset
            | StudioRecoveryDirective::ContinueRecording => {
                NativeStudioRecoveryAction::RecoverRecording
            }
            StudioRecoveryDirective::OpenEditor => NativeStudioRecoveryAction::OpenEditor,
            StudioRecoveryDirective::ReconcileEditSaveByDigest => {
                NativeStudioRecoveryAction::ReconcileEditSave
            }
            StudioRecoveryDirective::DeletePartialRenderThenOpenEditor
            | StudioRecoveryDirective::VerifyCommittedRenderThenOpenEditor
            | StudioRecoveryDirective::RequireOperatorDecision => {
                NativeStudioRecoveryAction::RequiresOperatorDecision
            }
        }
    }
}

pub(super) fn discover(
    projects: &RootedDir,
) -> Result<Vec<DiscoveredStudioProject>, NativeDesktopBackendError> {
    let mut names = projects
        .read_names_bounded(MAX_PROJECT_DIRECTORY_ENTRIES)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    names.sort();

    let mut journals = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for name in names {
        let Some(candidate) = CandidateName::parse(&name)? else {
            continue;
        };
        match candidate.kind {
            CandidateKind::Journal => {
                let mut file = projects
                    .open_regular_file(&candidate.relative)
                    .map_err(|_| NativeDesktopBackendError::Filesystem)?;
                let bytes = read_document(&mut file)?;
                let journal = StudioDocumentCodec::decode_journal(&bytes)
                    .map_err(|_| NativeDesktopBackendError::Filesystem)?;
                if journal.project_id != candidate.project_id
                    || journals
                        .insert(
                            candidate.project_id,
                            (candidate.identity, journal, candidate.relative.clone()),
                        )
                        .is_some()
                {
                    return Err(NativeDesktopBackendError::Filesystem);
                }
            }
            CandidateKind::Project => {
                let mut file = projects
                    .open_regular_file(&candidate.relative)
                    .map_err(|_| NativeDesktopBackendError::Filesystem)?;
                let bytes = read_document(&mut file)?;
                let manifest = StudioDocumentCodec::decode_project(&bytes)
                    .map_err(|_| NativeDesktopBackendError::Filesystem)?;
                if manifest.id != candidate.project_id
                    || manifests
                        .insert(candidate.project_id, (manifest, candidate.relative.clone()))
                        .is_some()
                {
                    return Err(NativeDesktopBackendError::Filesystem);
                }
            }
        }
        if journals.len().max(manifests.len()) > MAX_DISCOVERED_PROJECTS {
            return Err(NativeDesktopBackendError::Busy);
        }
    }

    if manifests
        .keys()
        .any(|project_id| !journals.contains_key(project_id))
    {
        return Err(NativeDesktopBackendError::Filesystem);
    }

    journals
        .into_iter()
        .map(
            |(project_id, (project_identity, journal, journal_relative))| {
                let manifest = manifests.remove(&project_id);
                let directive = recovery_directive(journal.boundary);
                Ok(DiscoveredStudioProject {
                    project_identity,
                    journal,
                    manifest: manifest.as_ref().map(|(project, _)| project.clone()),
                    journal_relative,
                    project_relative: manifest.map(|(_, path)| path),
                    directive,
                })
            },
        )
        .collect()
}

pub(super) fn authenticate_recovery(
    projects: &RootedDir,
    discovered: &DiscoveredStudioProject,
) -> Result<NativeStudioRecoveryAction, NativeDesktopBackendError> {
    let mut journal_file = projects
        .open_regular_file(&discovered.journal_relative)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let journal = StudioDocumentCodec::decode_journal(&read_document(&mut journal_file)?)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if journal != discovered.journal {
        return Err(NativeDesktopBackendError::StaleCatalog);
    }
    if let (Some(expected), Some(relative)) = (&discovered.manifest, &discovered.project_relative) {
        let mut project_file = projects
            .open_regular_file(relative)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        let manifest = StudioDocumentCodec::decode_project(&read_document(&mut project_file)?)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        if &manifest != expected {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
    }
    Ok(discovered.recovery_action())
}

pub(super) fn archive_current_journal(
    projects: &RootedDir,
    discovered: &DiscoveredStudioProject,
    expected: &StudioJournalSnapshot,
) -> Result<(), NativeDesktopBackendError> {
    let mut journal_file = projects
        .open_regular_file(&discovered.journal_relative)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let journal = StudioDocumentCodec::decode_journal(&read_document(&mut journal_file)?)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if &journal != expected || journal.project_id != discovered.project_id() {
        return Err(NativeDesktopBackendError::StaleCatalog);
    }
    let identity = journal_file.metadata().identity();
    let archive = match projects.create_private_dir(RECOVERY_ARCHIVE_DIRECTORY) {
        Ok(directory) => directory,
        Err(RootedIoError::EntryExists) => projects
            .open_dir(RECOVERY_ARCHIVE_DIRECTORY)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?,
        Err(_) => return Err(NativeDesktopBackendError::Filesystem),
    };
    archive
        .ensure_private_mode()
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    projects
        .publish_file_to_root_if_identity(
            &discovered.journal_relative,
            identity,
            &archive,
            &discovered.journal_relative,
        )
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    Ok(())
}

pub(super) fn authenticate_ready(
    projects: &RootedDir,
    discovered: &DiscoveredStudioProject,
) -> Result<(u64, u64), NativeDesktopBackendError> {
    let (manifest, duration_ms) = authenticate_ready_project(projects, discovered)?;
    Ok((manifest.revision, duration_ms))
}

pub(super) fn authenticate_ready_project(
    projects: &RootedDir,
    discovered: &DiscoveredStudioProject,
) -> Result<(StudioProjectManifest, u64), NativeDesktopBackendError> {
    if discovered.status() != NativeStudioProjectStatus::Ready {
        return Err(NativeDesktopBackendError::Unavailable);
    }
    let mut journal_file = projects
        .open_regular_file(&discovered.journal_relative)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let journal = StudioDocumentCodec::decode_journal(&read_document(&mut journal_file)?)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let project_relative = discovered
        .project_relative
        .as_ref()
        .ok_or(NativeDesktopBackendError::Filesystem)?;
    let mut project_file = projects
        .open_regular_file(project_relative)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let manifest = StudioDocumentCodec::decode_project(&read_document(&mut project_file)?)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if journal != discovered.journal
        || discovered.manifest.as_ref() != Some(&manifest)
        || !manifest_matches_open_boundary(&journal, &manifest)
        || recovery_directive(journal.boundary) != StudioRecoveryDirective::OpenEditor
    {
        return Err(NativeDesktopBackendError::StaleCatalog);
    }
    let duration_ms = project_duration_ms(&manifest)?;
    Ok((manifest, duration_ms))
}

fn manifest_matches_open_boundary(
    journal: &StudioJournalSnapshot,
    manifest: &StudioProjectManifest,
) -> bool {
    if journal.project_id != manifest.id || manifest.state != StudioState::Editing {
        return false;
    }
    match journal.boundary {
        JournalBoundary::RecordingStopped => {
            manifest.revision == 1 && manifest.edits == EditSpec::default()
        }
        JournalBoundary::EditSaveCommitted => {
            journal.pending_edit.as_ref().is_some_and(|pending| {
                pending
                    .expected_project_revision
                    .checked_add(1)
                    .is_some_and(|revision| revision == manifest.revision)
                    && pending.edits == manifest.edits
            })
        }
        _ => false,
    }
}

fn read_document(file: &mut RootedFile) -> Result<Vec<u8>, NativeDesktopBackendError> {
    let before = file.metadata();
    if before.size_bytes() == 0 || before.size_bytes() > MAX_STUDIO_DOCUMENT_BYTES as u64 {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    let capacity =
        usize::try_from(before.size_bytes()).map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.file_mut()
        .read_to_end(&mut bytes)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let after = file
        .refresh_metadata()
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if before != after || bytes.len() != capacity {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(bytes)
}

fn project_duration_ms(project: &StudioProjectManifest) -> Result<u64, NativeDesktopBackendError> {
    let mut maximum_ms = 0_u64;
    for asset in project
        .assets
        .iter()
        .filter(|asset| asset.track == TrackKind::Screen)
    {
        let end = asset
            .end()
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        maximum_ms = maximum_ms.max(exact_duration_ms(end)?);
    }
    if maximum_ms == 0 {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(maximum_ms)
}

fn exact_duration_ms(duration: ExactDuration) -> Result<u64, NativeDesktopBackendError> {
    let milliseconds = duration
        .numerator()
        .checked_mul(1_000)
        .map(|value| value.div_ceil(duration.denominator()))
        .ok_or(NativeDesktopBackendError::Filesystem)?;
    u64::try_from(milliseconds).map_err(|_| NativeDesktopBackendError::Filesystem)
}

#[derive(Debug, Clone, Copy)]
enum CandidateKind {
    Journal,
    Project,
}

struct CandidateName {
    kind: CandidateKind,
    identity: [u8; 16],
    project_id: StudioProjectId,
    relative: PathBuf,
}

impl CandidateName {
    fn parse(name: &OsStr) -> Result<Option<Self>, NativeDesktopBackendError> {
        let Some(text) = name.to_str() else {
            return Ok(None);
        };
        let (stem, kind) = if let Some(stem) = text.strip_suffix(JOURNAL_SUFFIX) {
            (stem, CandidateKind::Journal)
        } else if let Some(stem) = text.strip_suffix(PROJECT_SUFFIX) {
            (stem, CandidateKind::Project)
        } else {
            return Ok(None);
        };
        let identity = decode_identity(stem).ok_or(NativeDesktopBackendError::Filesystem)?;
        let project_id = StudioProjectId::from_csprng(identity)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        Ok(Some(Self {
            kind,
            identity,
            project_id,
            relative: Path::new(name).to_path_buf(),
        }))
    }
}

fn decode_identity(stem: &str) -> Option<[u8; 16]> {
    if stem.len() != 32
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    let mut decoded = [0_u8; 16];
    for (index, target) in decoded.iter_mut().enumerate() {
        let offset = index.checked_mul(2)?;
        let high = decode_hex(stem.as_bytes()[offset])?;
        let low = decode_hex(stem.as_bytes()[offset + 1])?;
        *target = high.checked_mul(16)?.checked_add(low)?;
    }
    Some(decoded)
}

const fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_names_require_exact_lowercase_storage_identity() {
        assert!(
            CandidateName::parse(OsStr::new(
                "000102030405060708090a0b0c0d0e0f.studio-journal"
            ))
            .expect("valid candidate")
            .is_some()
        );
        assert!(
            CandidateName::parse(OsStr::new("notes.txt"))
                .expect("unrelated entry")
                .is_none()
        );
        for invalid in [
            "00010203.studio-journal",
            "000102030405060708090A0b0c0d0e0f.studio-project",
            "000102030405060708090g0b0c0d0e0f.studio-project",
        ] {
            assert!(CandidateName::parse(OsStr::new(invalid)).is_err());
        }
    }

    #[test]
    fn exact_duration_rounds_up_without_floating_point() {
        assert_eq!(
            exact_duration_ms(ExactDuration::new(1, 30).expect("duration")).expect("milliseconds"),
            34
        );
        assert_eq!(
            exact_duration_ms(ExactDuration::new(48_000, 48_000).expect("duration"))
                .expect("milliseconds"),
            1_000
        );
    }
}
