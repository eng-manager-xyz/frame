//! Authenticated native Studio export preparation and execution.
//!
//! The desktop backend supplies a clean, revision-bound manifest. This module
//! re-compiles the canonical edit plan, re-verifies every immutable original,
//! and executes the same bounded native adapter used by media conformance.

use std::{fs::File, path::Path};

use frame_media::{
    AssetChecksum, CancellationToken, EncoderBackend, ExactDuration, FilesystemStudioOriginalStore,
    NativeExecutionError, NativeStudioAlignedFileSources, NativeStudioEditedExportArtifact,
    NativeStudioExportProfile, NativeStudioFileSource, NativeStudioRenderProgress,
    StudioProjectManifest, StudioTimelineCompiler, TimelineSource, TrackKind,
    render_studio_export_with_edits_preopened_for_backend_and_progress,
};

use crate::{
    ExportProfile, NativeDesktopBackendError,
    rooted_io::{FileIdentity, RootedDir, RootedFile},
};

use super::{map_rooted_io_error, sha256_rooted_file};

pub(super) struct PreparedStudioExport {
    project_revision: u64,
    sources: NativeStudioAlignedFileSources,
    originals: Vec<VerifiedStudioOriginal>,
    plan: frame_media::CanonicalEditPlan,
    profile: NativeStudioExportProfile,
}

struct VerifiedStudioOriginal {
    file: RootedFile,
    identity: FileIdentity,
    byte_len: u64,
    checksum: AssetChecksum,
}

impl std::fmt::Debug for PreparedStudioExport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedStudioExport")
            .field("project_revision", &self.project_revision)
            .field("source_count", &source_count(&self.sources))
            .field("plan_digest", &self.plan.digest())
            .field("profile", &self.profile)
            .finish()
    }
}

impl PreparedStudioExport {
    pub(super) fn prepare(
        studio_root: &Path,
        studio_directory: &RootedDir,
        manifest: &StudioProjectManifest,
        requested_profile: ExportProfile,
    ) -> Result<Self, NativeDesktopBackendError> {
        manifest.validate().map_err(map_studio_error)?;
        let timeline = TimelineSource::from_assets(&manifest.assets).map_err(map_studio_error)?;
        let plan = StudioTimelineCompiler::compile(&timeline, &manifest.edits)
            .map_err(map_studio_error)?;
        let profile = native_profile(requested_profile);
        let mut store =
            FilesystemStudioOriginalStore::new(studio_root).map_err(map_studio_error)?;
        let mut screen = None;
        let mut camera = None;
        let mut cursor = None;
        let mut microphone = None;
        let mut system_audio = None;
        let mut originals = Vec::with_capacity(manifest.assets.len());
        for asset in &manifest.assets {
            store
                .verified_original_path(manifest.id, asset)
                .map_err(map_studio_error)?;
            let relative = FilesystemStudioOriginalStore::durable_original_relative_path(
                manifest.id,
                asset.id,
            );
            let mut rooted = studio_directory
                .open_regular_file(relative)
                .map_err(map_rooted_io_error)?;
            let metadata = rooted.metadata();
            if metadata.size_bytes() != asset.byte_len
                || sha256_rooted_file(&mut rooted, metadata.identity(), metadata.size_bytes())?
                    != asset.checksum.to_hex()
            {
                return Err(NativeDesktopBackendError::Filesystem);
            }
            let source = NativeStudioFileSource {
                file: rooted
                    .file()
                    .try_clone()
                    .map_err(|_| NativeDesktopBackendError::Filesystem)?,
                timeline_start: rational_time_to_exact(asset.start)?,
                timeline_duration: rational_time_to_exact(asset.duration)?,
            };
            let slot = match asset.track {
                TrackKind::Screen => &mut screen,
                TrackKind::Camera => &mut camera,
                TrackKind::Cursor => &mut cursor,
                TrackKind::Microphone => &mut microphone,
                TrackKind::SystemAudio => &mut system_audio,
            };
            if slot.replace(source).is_some() {
                return Err(NativeDesktopBackendError::InvalidEdit);
            }
            originals.push(VerifiedStudioOriginal {
                file: rooted,
                identity: metadata.identity(),
                byte_len: metadata.size_bytes(),
                checksum: asset.checksum,
            });
        }
        Ok(Self {
            project_revision: manifest.revision,
            sources: NativeStudioAlignedFileSources {
                screen: screen.ok_or(NativeDesktopBackendError::InvalidEdit)?,
                camera,
                cursor,
                microphone,
                system_audio,
            },
            originals,
            plan,
            profile,
        })
    }

    pub(super) const fn extension(&self) -> &'static str {
        self.profile.extension()
    }

    pub(super) fn render_preopened(
        self,
        artifact_path: &Path,
        output: File,
        cancellation: &CancellationToken,
    ) -> Result<NativeStudioEditedExportArtifact, NativeDesktopBackendError> {
        self.render_preopened_with_backend_and_progress(
            artifact_path,
            output,
            EncoderBackend::Software,
            cancellation,
            |_| {},
        )
    }

    pub(super) fn render_preopened_with_backend_and_progress(
        mut self,
        artifact_path: &Path,
        output: File,
        backend: EncoderBackend,
        cancellation: &CancellationToken,
        progress: impl FnMut(NativeStudioRenderProgress),
    ) -> Result<NativeStudioEditedExportArtifact, NativeDesktopBackendError> {
        let artifact = render_studio_export_with_edits_preopened_for_backend_and_progress(
            self.sources,
            artifact_path,
            output,
            &self.plan,
            self.profile,
            backend,
            cancellation,
            progress,
        )
        .map_err(map_native_error)?;
        for original in &mut self.originals {
            if sha256_rooted_file(&mut original.file, original.identity, original.byte_len)?
                != original.checksum.to_hex()
            {
                return Err(NativeDesktopBackendError::Filesystem);
            }
        }
        Ok(artifact)
    }
}

fn rational_time_to_exact(
    value: frame_media::RationalTime,
) -> Result<ExactDuration, NativeDesktopBackendError> {
    ExactDuration::new(
        u128::from(value.ticks()),
        u128::from(value.time_base().ticks_per_second()),
    )
    .map_err(|_| NativeDesktopBackendError::InvalidEdit)
}

fn native_profile(profile: ExportProfile) -> NativeStudioExportProfile {
    match profile {
        ExportProfile::DistributionMp4 => NativeStudioExportProfile::DistributionMasterMp4,
        ExportProfile::EditableWebm => NativeStudioExportProfile::EditableWebM,
        ExportProfile::Archive => NativeStudioExportProfile::NativeArchiveMatroska,
    }
}

fn source_count(sources: &NativeStudioAlignedFileSources) -> usize {
    1 + usize::from(sources.camera.is_some())
        + usize::from(sources.cursor.is_some())
        + usize::from(sources.microphone.is_some())
        + usize::from(sources.system_audio.is_some())
}

fn map_studio_error(error: frame_media::StudioError) -> NativeDesktopBackendError {
    match error {
        frame_media::StudioError::StorageIo | frame_media::StudioError::UnsafeStoragePath => {
            NativeDesktopBackendError::Filesystem
        }
        _ => NativeDesktopBackendError::InvalidEdit,
    }
}

pub(super) fn map_native_error(error: NativeExecutionError) -> NativeDesktopBackendError {
    match error {
        NativeExecutionError::Cancelled => NativeDesktopBackendError::Cancelled,
        NativeExecutionError::Filesystem | NativeExecutionError::InvalidOutput => {
            NativeDesktopBackendError::Filesystem
        }
        NativeExecutionError::InvalidGraph
        | NativeExecutionError::NoSources
        | NativeExecutionError::ResourceLimit => NativeDesktopBackendError::InvalidEdit,
        NativeExecutionError::MissingFactory
        | NativeExecutionError::UntrustedFactory
        | NativeExecutionError::CodecApprovalRequired => NativeDesktopBackendError::Unavailable,
        NativeExecutionError::Pipeline | NativeExecutionError::Timeout => {
            NativeDesktopBackendError::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use frame_media::{
        AssetChecksum, AssetCommitState, AudioSampleFormat, EditSpec, FrameRate, MediaContainer,
        NativeStudioTrackRole, PixelFormat, STUDIO_ASSET_VERSION, STUDIO_PROJECT_VERSION,
        StudioAsset, StudioAssetCodec, StudioAssetEncoding, StudioAssetId, StudioAssetRawCaps,
        StudioAudioCodec, StudioAudioRawCaps, StudioOperationId, StudioProjectId, StudioSourceName,
        StudioState, StudioVideoCodec, StudioVideoRawCaps, TempAssetCommitTicket, TimeBase,
        commit_verified_temporary, record_synthetic_studio_tracks,
    };

    use super::*;

    #[test]
    fn profile_mapping_is_explicit_and_complete() {
        assert_eq!(
            native_profile(ExportProfile::DistributionMp4),
            NativeStudioExportProfile::DistributionMasterMp4
        );
        assert_eq!(
            native_profile(ExportProfile::EditableWebm),
            NativeStudioExportProfile::EditableWebM
        );
        assert_eq!(
            native_profile(ExportProfile::Archive),
            NativeStudioExportProfile::NativeArchiveMatroska
        );
    }

    #[test]
    fn prepared_export_reverifies_originals_and_renders_nonzero_timeline_range() {
        let directory = tempfile::tempdir().expect("fixture");
        let encoded = record_synthetic_studio_tracks(
            &directory.path().join("encoded"),
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .expect("synthetic isolated tracks");
        let track = |role| {
            encoded
                .iter()
                .find(|track| track.role == role)
                .expect("requested role")
                .path
                .clone()
        };
        let project_id = StudioProjectId::from_csprng([1; 16]).expect("project ID");
        let fixture_root = fs::canonicalize(directory.path()).expect("canonical fixture");
        let studio_root = fixture_root.join("studio");
        let mut store = FilesystemStudioOriginalStore::new(&studio_root).expect("original store");
        let screen = commit_asset(
            &mut store,
            project_id,
            &track(NativeStudioTrackRole::Screen),
            2,
            TrackKind::Screen,
            0,
            2,
        );
        let system_audio = commit_asset(
            &mut store,
            project_id,
            &track(NativeStudioTrackRole::SystemAudio),
            3,
            TrackKind::SystemAudio,
            1,
            1,
        );
        let manifest = StudioProjectManifest {
            version: STUDIO_PROJECT_VERSION,
            id: project_id,
            revision: 1,
            state: StudioState::Editing,
            assets: vec![screen, system_audio],
            edits: EditSpec::default(),
        };
        let studio_directory = RootedDir::bind(&studio_root).expect("rooted Studio directory");
        let prepared = PreparedStudioExport::prepare(
            &studio_root,
            &studio_directory,
            &manifest,
            ExportProfile::EditableWebm,
        )
        .expect("authenticated export");
        let detached_studio_root = fixture_root.join("detached-studio");
        fs::rename(&studio_root, &detached_studio_root).expect("detach visible Studio root");
        fs::create_dir(&studio_root).expect("replace visible Studio root");
        let output = directory.path().join("edited.webm");
        let output_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&output)
            .expect("preopened export");
        let artifact = prepared
            .render_preopened(&output, output_file, &CancellationToken::new())
            .expect("native edit-aware render");
        assert_eq!(artifact.path, output);
        assert!(artifact.bytes > 0);
        assert_eq!(artifact.sha256.len(), 64);
        assert!(output.is_file());
        assert!(detached_studio_root.is_dir());
    }

    fn commit_asset(
        store: &mut FilesystemStudioOriginalStore,
        project_id: StudioProjectId,
        source: &Path,
        marker: u8,
        track: TrackKind,
        start_ticks: u64,
        duration_ticks: u64,
    ) -> StudioAsset {
        let bytes = fs::read(source).expect("encoded source");
        let time_base = TimeBase::new(1).expect("time base");
        let encoding = match track {
            TrackKind::Screen => StudioAssetEncoding::Encoded {
                container: MediaContainer::WebM,
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
                time_base: TimeBase::new(90_000).expect("video time base"),
            },
            TrackKind::SystemAudio => StudioAssetEncoding::Encoded {
                container: MediaContainer::WebM,
                codec: StudioAssetCodec::Audio(StudioAudioCodec::Opus),
                raw_caps: StudioAssetRawCaps::Audio(StudioAudioRawCaps {
                    sample_rate: 48_000,
                    channels: 2,
                    sample_format: AudioSampleFormat::Float32,
                }),
                time_base: TimeBase::new(48_000).expect("audio time base"),
            },
            TrackKind::Camera | TrackKind::Microphone | TrackKind::Cursor => {
                unreachable!("fixture track")
            }
        };
        let temporary = StudioAsset {
            version: STUDIO_ASSET_VERSION,
            id: StudioAssetId::from_csprng([marker; 16]).expect("asset ID"),
            track,
            source_name: StudioSourceName::new(format!("{track:?}.webm").to_ascii_lowercase())
                .expect("source name"),
            byte_len: u64::try_from(bytes.len()).expect("asset length"),
            start: frame_media::RationalTime::new(start_ticks, time_base),
            duration: frame_media::RationalTime::new(duration_ticks, time_base),
            checksum: AssetChecksum::from_content(&bytes),
            commit_state: AssetCommitState::Temporary,
            encoding,
        };
        store
            .stage_temporary_bytes(project_id, &temporary, &bytes)
            .expect("stage original");
        commit_verified_temporary(
            store,
            TempAssetCommitTicket::new(
                project_id,
                StudioOperationId::from_csprng([marker + 10; 16]).expect("operation ID"),
                1,
                temporary,
            )
            .expect("commit ticket"),
        )
        .expect("durable original")
    }
}
