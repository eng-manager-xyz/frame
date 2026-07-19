#![forbid(unsafe_code)]

//! Durable, descriptor-rooted A/V settings and installation-secret storage.
//!
//! Settings use two fixed revision slots. A commit writes and `fsync`s a
//! private staging file, publishes it with a no-replace descriptor-relative
//! rename, and only then returns the new revision. The other slot always keeps
//! the prior durable revision, so interruption before publication cannot erase
//! backend truth and the directory never grows with revision count.

use std::{
    fmt,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
};

use frame_media::{
    AV_SETTINGS_VERSION, AvCaptureError, AvCaptureSettingsV2, AvDeviceCatalog, AvDeviceId,
    AvSettingsCodec, AvSourceClass, DeviceSelectionV2, MAX_PERSISTED_AV_SETTINGS_BYTES,
    SelectionResolution, resolve_selection,
};
use ring::rand::{SecureRandom, SystemRandom};
use rustix::io::Errno;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::rooted_io::{FileIdentity, RootedDir, RootedFile, RootedIoError};

/// Fixed private directory beneath the desktop app-data root.
pub const AV_SETTINGS_DIRECTORY: &str = "av-settings";

/// Exact maximum number of settings bytes read from one descriptor.
pub const AV_SETTINGS_READ_LIMIT_BYTES: usize = 4_096;

/// The in-memory screen-only baseline before the first durable mutation.
pub const INITIAL_AV_SETTINGS_REVISION: u64 = 1;

const PERSISTED_SCHEMA_VERSION: u16 = 1;
const FIRST_PERSISTED_REVISION: u64 = INITIAL_AV_SETTINGS_REVISION + 1;
const SETTINGS_SLOT_NAMES: [&str; 2] = ["settings-a.json", "settings-b.json"];
const SETTINGS_STAGING_NAME: &str = ".settings-staging";
const INSTALLATION_SECRET_NAME: &str = "installation-secret";
const INSTALLATION_SECRET_STAGING_NAME: &str = ".installation-secret-staging";
const INSTALLATION_SECRET_BYTES: usize = 32;
const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedSettingsEnvelope {
    schema_version: u16,
    revision: u64,
    encoded_settings: String,
}

/// One backend-confirmed settings revision.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DurableAvSettingsSnapshot {
    revision: u64,
    settings: AvCaptureSettingsV2,
}

impl DurableAvSettingsSnapshot {
    /// Durable compare-and-swap revision.
    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    /// Exact persisted selections. Device labels are never stored.
    #[must_use]
    pub const fn settings(self) -> AvCaptureSettingsV2 {
        self.settings
    }

    /// Resolve one persisted selection while preserving a missing pinned ID.
    ///
    /// A pinned device is never replaced with the catalog default. Default
    /// resolution occurs only for an explicit `FollowDefault` selection.
    pub fn selection_status(
        self,
        catalog: &AvDeviceCatalog,
        class: AvSourceClass,
    ) -> Result<DurableSelectionStatus, AvCaptureError> {
        let selection = self.settings.selection(class);
        match selection {
            DeviceSelectionV2::Disabled => Ok(DurableSelectionStatus::Disabled),
            DeviceSelectionV2::Pinned { id, .. } => {
                if !catalog
                    .devices()
                    .iter()
                    .any(|device| device.class() == class && device.id() == id)
                {
                    return Ok(DurableSelectionStatus::PinnedMissing { id });
                }
                Ok(DurableSelectionStatus::Pinned {
                    id,
                    resolution: resolve_selection(catalog, class, selection)?,
                })
            }
            DeviceSelectionV2::FollowDefault { .. } => Ok(DurableSelectionStatus::FollowDefault {
                resolution: resolve_selection(catalog, class, selection)?,
            }),
        }
    }
}

impl fmt::Debug for DurableAvSettingsSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableAvSettingsSnapshot")
            .field("revision", &self.revision)
            .field("settings", &"<redacted>")
            .finish()
    }
}

/// Availability of one exact persisted selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableSelectionStatus {
    Disabled,
    PinnedMissing {
        id: AvDeviceId,
    },
    Pinned {
        id: AvDeviceId,
        resolution: SelectionResolution,
    },
    FollowDefault {
        resolution: SelectionResolution,
    },
}

/// Exactly 32 secret bytes whose debug representation is always redacted.
pub struct InstallationSecret {
    bytes: Zeroizing<[u8; INSTALLATION_SECRET_BYTES]>,
}

impl InstallationSecret {
    /// Borrow the secret only at the native identifier-derivation boundary.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; INSTALLATION_SECRET_BYTES] {
        &self.bytes
    }
}

impl fmt::Debug for InstallationSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InstallationSecret(<redacted>)")
    }
}

/// Fail-closed storage errors. No path, device ID, settings payload, or secret
/// is retained in an error variant.
#[derive(Debug, Error)]
pub enum DurableAvSettingsError {
    #[error("descriptor-rooted A/V settings storage failed")]
    Rooted(#[source] RootedIoError),
    #[error("A/V settings file access failed")]
    FileAccess,
    #[error("A/V settings file does not have exact mode 0600")]
    InsecureFileMode,
    #[error("A/V settings exceed the exact 4 KiB storage limit")]
    SettingsTooLarge,
    #[error("A/V settings were truncated or changed while being read")]
    TruncatedOrChanged,
    #[error("A/V settings JSON is malformed or contains an unknown field")]
    MalformedSettings,
    #[error("A/V settings schema version is unsupported")]
    UnsupportedSchemaVersion,
    #[error("A/V settings contain an invalid media selection")]
    InvalidSettings,
    #[error("A/V settings slots do not form one bounded revision sequence")]
    ConflictingRevisions,
    #[error("A/V settings revision {expected} is stale; durable revision is {current}")]
    StaleRevision { expected: u64, current: u64 },
    #[error("A/V settings revision exhausted its range")]
    RevisionExhausted,
    #[error("installation secret must be exactly 32 bytes")]
    InvalidSecretLength,
    #[error("installation secret is invalid")]
    InvalidInstallationSecret,
    #[error("cryptographically secure installation-secret generation failed")]
    SecretGeneration,
}

impl From<RootedIoError> for DurableAvSettingsError {
    fn from(error: RootedIoError) -> Self {
        Self::Rooted(error)
    }
}

#[derive(Clone, Copy)]
struct SlotRecord {
    snapshot: DurableAvSettingsSnapshot,
    identity: FileIdentity,
}

struct LoadedSettings {
    current: DurableAvSettingsSnapshot,
    slots: [Option<SlotRecord>; SETTINGS_SLOT_NAMES.len()],
}

#[derive(Clone, Copy)]
struct StagedFile {
    identity: FileIdentity,
    bytes: u64,
}

/// Descriptor-rooted authority for durable A/V settings and installation ID
/// material.
///
/// The native adapter must own one writer and serialize mutations through it.
/// Descriptor-rooted publication is crash-safe but is not a cross-process lock.
pub struct DurableAvSettingsStore {
    directory: RootedDir,
}

impl DurableAvSettingsStore {
    /// Open or create the fixed private settings directory beneath a trusted
    /// app-data descriptor.
    pub fn open(app_data: &RootedDir) -> Result<Self, DurableAvSettingsError> {
        let directory = match app_data.create_private_dir(AV_SETTINGS_DIRECTORY) {
            Ok(directory) => directory,
            Err(RootedIoError::EntryExists) => app_data.open_dir(AV_SETTINGS_DIRECTORY)?,
            Err(error) => return Err(error.into()),
        };
        directory.ensure_private_mode()?;
        Ok(Self { directory })
    }

    /// Load the highest complete revision. Before the first durable write this
    /// returns the revision-one, screen-only baseline.
    pub fn load(&self) -> Result<DurableAvSettingsSnapshot, DurableAvSettingsError> {
        Ok(self.load_state()?.current)
    }

    /// Atomically persist exactly `expected_revision + 1`.
    ///
    /// Callers must update backend/UI truth only from the returned snapshot.
    /// Any error leaves the prior returned revision authoritative; a process
    /// restart or lost acknowledgement is recovered by calling [`Self::load`].
    pub fn compare_and_swap(
        &mut self,
        expected_revision: u64,
        settings: AvCaptureSettingsV2,
    ) -> Result<DurableAvSettingsSnapshot, DurableAvSettingsError> {
        let settings = settings
            .validate()
            .map_err(|_| DurableAvSettingsError::InvalidSettings)?;
        let loaded = self.load_state()?;
        if loaded.current.revision != expected_revision {
            return Err(DurableAvSettingsError::StaleRevision {
                expected: expected_revision,
                current: loaded.current.revision,
            });
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or(DurableAvSettingsError::RevisionExhausted)?;
        let next = DurableAvSettingsSnapshot { revision, settings };
        let encoded = encode_snapshot(next)?;
        let target_slot = slot_for_revision(revision);

        self.cleanup_regular_if_present(SETTINGS_STAGING_NAME)?;
        self.verify_current_identity(&loaded)?;
        if let Some(stale) = loaded.slots[target_slot] {
            self.directory
                .cleanup_file_if_identity(SETTINGS_SLOT_NAMES[target_slot], stale.identity)?;
        }

        let staged = self.stage_bytes(SETTINGS_STAGING_NAME, &encoded)?;
        let publication = self.directory.publish_file_if_identity(
            SETTINGS_STAGING_NAME,
            staged.identity,
            SETTINGS_SLOT_NAMES[target_slot],
        );
        let published = match publication {
            Ok(published) => published,
            Err(error) => {
                let _ = self
                    .directory
                    .cleanup_file_if_identity(SETTINGS_STAGING_NAME, staged.identity);
                return Err(error.into());
            }
        };
        if published.identity() != staged.identity || published.size_bytes() != staged.bytes {
            return Err(DurableAvSettingsError::TruncatedOrChanged);
        }
        Ok(next)
    }

    /// Load the existing installation secret without creating one.
    pub fn load_installation_secret(
        &self,
    ) -> Result<Option<InstallationSecret>, DurableAvSettingsError> {
        let Some(file) = self.open_optional_regular(INSTALLATION_SECRET_NAME)? else {
            return Ok(None);
        };
        read_installation_secret(file).map(Some)
    }

    /// Load the stable installation secret or securely create it once.
    pub fn load_or_create_installation_secret(
        &mut self,
    ) -> Result<InstallationSecret, DurableAvSettingsError> {
        if let Some(secret) = self.load_installation_secret()? {
            return Ok(secret);
        }
        let mut bytes = Zeroizing::new([0_u8; INSTALLATION_SECRET_BYTES]);
        SystemRandom::new()
            .fill(bytes.as_mut())
            .map_err(|_| DurableAvSettingsError::SecretGeneration)?;
        self.create_installation_secret(bytes)
    }

    fn create_installation_secret(
        &mut self,
        bytes: Zeroizing<[u8; INSTALLATION_SECRET_BYTES]>,
    ) -> Result<InstallationSecret, DurableAvSettingsError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(DurableAvSettingsError::InvalidInstallationSecret);
        }
        self.cleanup_regular_if_present(INSTALLATION_SECRET_STAGING_NAME)?;
        let staged = self.stage_bytes(INSTALLATION_SECRET_STAGING_NAME, bytes.as_ref())?;
        let publication = self.directory.publish_file_if_identity(
            INSTALLATION_SECRET_STAGING_NAME,
            staged.identity,
            INSTALLATION_SECRET_NAME,
        );
        match publication {
            Ok(published)
                if published.identity() == staged.identity
                    && published.size_bytes() == staged.bytes =>
            {
                Ok(InstallationSecret { bytes })
            }
            Ok(_) => Err(DurableAvSettingsError::TruncatedOrChanged),
            Err(error) => {
                let _ = self
                    .directory
                    .cleanup_file_if_identity(INSTALLATION_SECRET_STAGING_NAME, staged.identity);
                // A concurrent first writer may have won the no-replace rename.
                // Its exact durable value is authoritative.
                if matches!(
                    error,
                    RootedIoError::DestinationExists | RootedIoError::EntryExists
                ) && let Some(secret) = self.load_installation_secret()?
                {
                    return Ok(secret);
                }
                Err(error.into())
            }
        }
    }

    fn load_state(&self) -> Result<LoadedSettings, DurableAvSettingsError> {
        let slots = [self.read_slot(0)?, self.read_slot(1)?];
        let current = match (slots[0], slots[1]) {
            (None, None) => DurableAvSettingsSnapshot {
                revision: INITIAL_AV_SETTINGS_REVISION,
                settings: AvCaptureSettingsV2::screen_only(),
            },
            (Some(record), None) | (None, Some(record)) => record.snapshot,
            (Some(left), Some(right)) => {
                if left.snapshot.revision.abs_diff(right.snapshot.revision) != 1 {
                    return Err(DurableAvSettingsError::ConflictingRevisions);
                }
                if left.snapshot.revision > right.snapshot.revision {
                    left.snapshot
                } else {
                    right.snapshot
                }
            }
        };
        Ok(LoadedSettings { current, slots })
    }

    fn read_slot(&self, slot: usize) -> Result<Option<SlotRecord>, DurableAvSettingsError> {
        let Some(file) = self.open_optional_regular(SETTINGS_SLOT_NAMES[slot])? else {
            return Ok(None);
        };
        let identity = file.metadata().identity();
        let bytes = read_private_file(file, AV_SETTINGS_READ_LIMIT_BYTES)?;
        let snapshot = decode_snapshot(&bytes)?;
        if snapshot.revision < FIRST_PERSISTED_REVISION
            || slot_for_revision(snapshot.revision) != slot
        {
            return Err(DurableAvSettingsError::ConflictingRevisions);
        }
        Ok(Some(SlotRecord { snapshot, identity }))
    }

    fn open_optional_regular(
        &self,
        name: &str,
    ) -> Result<Option<RootedFile>, DurableAvSettingsError> {
        match self.directory.open_regular_file(name) {
            Ok(file) => Ok(Some(file)),
            Err(RootedIoError::Io {
                source: Errno::NOENT,
                ..
            }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn cleanup_regular_if_present(&self, name: &str) -> Result<(), DurableAvSettingsError> {
        let Some(file) = self.open_optional_regular(name)? else {
            return Ok(());
        };
        self.directory
            .cleanup_file_if_identity(name, file.metadata().identity())?;
        Ok(())
    }

    fn verify_current_identity(
        &self,
        loaded: &LoadedSettings,
    ) -> Result<(), DurableAvSettingsError> {
        if loaded.current.revision == INITIAL_AV_SETTINGS_REVISION {
            if self
                .open_optional_regular(SETTINGS_SLOT_NAMES[0])?
                .is_some()
                || self
                    .open_optional_regular(SETTINGS_SLOT_NAMES[1])?
                    .is_some()
            {
                return Err(DurableAvSettingsError::ConflictingRevisions);
            }
            return Ok(());
        }
        let slot = slot_for_revision(loaded.current.revision);
        let expected = loaded.slots[slot].ok_or(DurableAvSettingsError::ConflictingRevisions)?;
        let current = self
            .open_optional_regular(SETTINGS_SLOT_NAMES[slot])?
            .ok_or(DurableAvSettingsError::ConflictingRevisions)?;
        if current.metadata().identity() != expected.identity {
            return Err(DurableAvSettingsError::ConflictingRevisions);
        }
        Ok(())
    }

    fn stage_bytes(&self, name: &str, bytes: &[u8]) -> Result<StagedFile, DurableAvSettingsError> {
        self.cleanup_regular_if_present(name)?;
        let mut file = self.directory.create_new_file(name)?;
        let identity = file.metadata().identity();
        let result = (|| {
            file.file_mut()
                .write_all(bytes)
                .map_err(|_| DurableAvSettingsError::FileAccess)?;
            file.sync()?;
            let refreshed = file.refresh_metadata()?;
            if refreshed.identity() != identity
                || refreshed.size_bytes()
                    != u64::try_from(bytes.len())
                        .map_err(|_| DurableAvSettingsError::SettingsTooLarge)?
            {
                return Err(DurableAvSettingsError::TruncatedOrChanged);
            }
            ensure_private_file_mode(&file)?;
            Ok(StagedFile {
                identity,
                bytes: refreshed.size_bytes(),
            })
        })();
        if result.is_err() {
            drop(file);
            let _ = self.directory.cleanup_file_if_identity(name, identity);
        }
        result
    }
}

impl fmt::Debug for DurableAvSettingsStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableAvSettingsStore")
            .field("directory", &"<redacted>")
            .finish()
    }
}

fn slot_for_revision(revision: u64) -> usize {
    if revision.is_multiple_of(2) { 0 } else { 1 }
}

fn encode_snapshot(snapshot: DurableAvSettingsSnapshot) -> Result<Vec<u8>, DurableAvSettingsError> {
    let encoded_settings = AvSettingsCodec::encode(snapshot.settings)
        .map_err(|_| DurableAvSettingsError::InvalidSettings)?;
    let encoded_settings =
        String::from_utf8(encoded_settings).map_err(|_| DurableAvSettingsError::InvalidSettings)?;
    let envelope = PersistedSettingsEnvelope {
        schema_version: PERSISTED_SCHEMA_VERSION,
        revision: snapshot.revision,
        encoded_settings,
    };
    let encoded =
        serde_json::to_vec(&envelope).map_err(|_| DurableAvSettingsError::MalformedSettings)?;
    if encoded.len() > AV_SETTINGS_READ_LIMIT_BYTES {
        return Err(DurableAvSettingsError::SettingsTooLarge);
    }
    Ok(encoded)
}

fn decode_snapshot(bytes: &[u8]) -> Result<DurableAvSettingsSnapshot, DurableAvSettingsError> {
    let envelope: PersistedSettingsEnvelope =
        serde_json::from_slice(bytes).map_err(|_| DurableAvSettingsError::MalformedSettings)?;
    if envelope.schema_version != PERSISTED_SCHEMA_VERSION {
        return Err(DurableAvSettingsError::UnsupportedSchemaVersion);
    }
    if envelope.encoded_settings.len() > MAX_PERSISTED_AV_SETTINGS_BYTES {
        return Err(DurableAvSettingsError::SettingsTooLarge);
    }
    let settings = AvSettingsCodec::decode(envelope.encoded_settings.as_bytes())
        .map_err(|_| DurableAvSettingsError::InvalidSettings)?;
    if settings.version != AV_SETTINGS_VERSION {
        return Err(DurableAvSettingsError::InvalidSettings);
    }
    Ok(DurableAvSettingsSnapshot {
        revision: envelope.revision,
        settings,
    })
}

fn read_private_file(
    mut file: RootedFile,
    max_bytes: usize,
) -> Result<Vec<u8>, DurableAvSettingsError> {
    ensure_private_file_mode(&file)?;
    let declared = usize::try_from(file.metadata().size_bytes())
        .map_err(|_| DurableAvSettingsError::SettingsTooLarge)?;
    if declared > max_bytes {
        return Err(DurableAvSettingsError::SettingsTooLarge);
    }
    let mut bytes = vec![0_u8; declared];
    file.file_mut()
        .read_exact(&mut bytes)
        .map_err(|_| DurableAvSettingsError::TruncatedOrChanged)?;
    let refreshed = file.refresh_metadata()?;
    if refreshed.size_bytes()
        != u64::try_from(declared).map_err(|_| DurableAvSettingsError::SettingsTooLarge)?
    {
        return Err(DurableAvSettingsError::TruncatedOrChanged);
    }
    Ok(bytes)
}

fn read_installation_secret(
    mut file: RootedFile,
) -> Result<InstallationSecret, DurableAvSettingsError> {
    ensure_private_file_mode(&file)?;
    if file.metadata().size_bytes()
        != u64::try_from(INSTALLATION_SECRET_BYTES)
            .map_err(|_| DurableAvSettingsError::InvalidSecretLength)?
    {
        return Err(DurableAvSettingsError::InvalidSecretLength);
    }
    let mut bytes = Zeroizing::new([0_u8; INSTALLATION_SECRET_BYTES]);
    file.file_mut()
        .read_exact(bytes.as_mut())
        .map_err(|_| DurableAvSettingsError::TruncatedOrChanged)?;
    if file.refresh_metadata()?.size_bytes()
        != u64::try_from(INSTALLATION_SECRET_BYTES)
            .map_err(|_| DurableAvSettingsError::InvalidSecretLength)?
    {
        return Err(DurableAvSettingsError::TruncatedOrChanged);
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(DurableAvSettingsError::InvalidInstallationSecret);
    }
    Ok(InstallationSecret { bytes })
}

fn ensure_private_file_mode(file: &RootedFile) -> Result<(), DurableAvSettingsError> {
    let metadata = file
        .file()
        .metadata()
        .map_err(|_| DurableAvSettingsError::FileAccess)?;
    if metadata.permissions().mode() & 0o7777 != PRIVATE_FILE_MODE {
        return Err(DurableAvSettingsError::InsecureFileMode);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use frame_media::{
        AudioFormat, AudioSampleFormat, AvAdapterInstanceId, AvDeviceDescriptor,
        AvDeviceGeneration, AvFormat, NativeRouteClass, NativeTimestampKind, PermissionState,
    };
    use serde_json::json;

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "frame-av-settings-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create settings test directory");
            let path = fs::canonicalize(path).expect("canonicalize settings test directory");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct Fixture {
        temporary: TestDirectory,
        parent: RootedDir,
        store: DurableAvSettingsStore,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let temporary = TestDirectory::new(label);
            let parent = RootedDir::bind(&temporary.path).expect("bind test app-data root");
            let store = DurableAvSettingsStore::open(&parent).expect("open durable settings");
            Self {
                temporary,
                parent,
                store,
            }
        }

        fn settings_path(&self) -> PathBuf {
            self.temporary.path.join(AV_SETTINGS_DIRECTORY)
        }
    }

    fn audio_format() -> AvFormat {
        AvFormat::Audio(AudioFormat {
            sample_rate: 48_000,
            channels: 2,
            sample_format: AudioSampleFormat::Float32,
        })
    }

    fn pinned_settings(marker: u8) -> AvCaptureSettingsV2 {
        AvCaptureSettingsV2 {
            version: AV_SETTINGS_VERSION,
            microphone: DeviceSelectionV2::Disabled,
            system_audio: DeviceSelectionV2::Pinned {
                id: AvDeviceId::from_opaque([marker; 16]).expect("test device ID"),
                format: audio_format(),
            },
            camera: DeviceSelectionV2::Disabled,
        }
    }

    fn catalog_with_default(marker: u8) -> AvDeviceCatalog {
        let device = AvDeviceDescriptor::new(
            AvDeviceId::from_opaque([marker; 16]).expect("catalog device ID"),
            AvDeviceGeneration::new(1).expect("device generation"),
            AvSourceClass::SystemAudio,
            true,
            PermissionState::Granted,
            NativeRouteClass::BuiltIn,
            NativeTimestampKind::HostMonotonic,
            vec![audio_format()],
        )
        .expect("catalog descriptor");
        AvDeviceCatalog::new(
            AvAdapterInstanceId::from_opaque([9; 16]).expect("catalog adapter"),
            1,
            vec![device],
        )
        .expect("device catalog")
    }

    fn write_private(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(path)
            .expect("create private fixture file");
        file.write_all(bytes).expect("write private fixture file");
        file.sync_all().expect("sync private fixture file");
        fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
            .expect("set private fixture mode");
    }

    #[test]
    fn cas_round_trip_uses_exact_revision_and_private_modes() {
        let mut fixture = Fixture::new("cas");
        let baseline = fixture.store.load().expect("load baseline");
        assert_eq!(baseline.revision(), INITIAL_AV_SETTINGS_REVISION);
        assert_eq!(baseline.settings(), AvCaptureSettingsV2::screen_only());

        let committed = fixture
            .store
            .compare_and_swap(baseline.revision(), pinned_settings(1))
            .expect("commit settings");
        assert_eq!(committed.revision(), 2);
        assert_eq!(fixture.store.load().expect("reload settings"), committed);
        assert!(matches!(
            fixture
                .store
                .compare_and_swap(INITIAL_AV_SETTINGS_REVISION, pinned_settings(2)),
            Err(DurableAvSettingsError::StaleRevision {
                expected: 1,
                current: 2,
            })
        ));

        let reopened = DurableAvSettingsStore::open(&fixture.parent).expect("reopen settings");
        assert_eq!(
            reopened.load().expect("reload reopened settings"),
            committed
        );
        let directory_mode = fs::metadata(fixture.settings_path())
            .expect("settings directory metadata")
            .permissions()
            .mode()
            & 0o7777;
        let file_mode = fs::metadata(fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]))
            .expect("settings file metadata")
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode, PRIVATE_FILE_MODE);
        assert!(!format!("{committed:?}").contains("01010101"));
    }

    #[test]
    fn missing_pinned_device_remains_explicit_and_never_uses_default() {
        let mut fixture = Fixture::new("missing-device");
        let committed = fixture
            .store
            .compare_and_swap(INITIAL_AV_SETTINGS_REVISION, pinned_settings(1))
            .expect("commit missing-device selection");
        let status = committed
            .selection_status(&catalog_with_default(2), AvSourceClass::SystemAudio)
            .expect("resolve persisted selection");
        assert_eq!(
            status,
            DurableSelectionStatus::PinnedMissing {
                id: AvDeviceId::from_opaque([1; 16]).expect("missing ID"),
            }
        );
    }

    #[test]
    fn strict_json_rejects_unknown_fields_and_unbounded_ids() {
        let fixture = Fixture::new("strict-json");
        let mut value: serde_json::Value = serde_json::from_slice(
            &encode_snapshot(DurableAvSettingsSnapshot {
                revision: 2,
                settings: pinned_settings(1),
            })
            .expect("encode fixture"),
        )
        .expect("decode fixture JSON");
        value
            .as_object_mut()
            .expect("settings envelope object")
            .insert("device_name".to_owned(), json!("must-not-persist"));
        write_private(
            &fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
            &serde_json::to_vec(&value).expect("encode unknown field"),
        );
        assert!(matches!(
            fixture.store.load(),
            Err(DurableAvSettingsError::MalformedSettings)
        ));

        let fixture = Fixture::new("unsupported-schema");
        value
            .as_object_mut()
            .expect("settings envelope object")
            .remove("device_name");
        value["schema_version"] = json!(PERSISTED_SCHEMA_VERSION + 1);
        write_private(
            &fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
            &serde_json::to_vec(&value).expect("encode unsupported schema"),
        );
        assert!(matches!(
            fixture.store.load(),
            Err(DurableAvSettingsError::UnsupportedSchemaVersion)
        ));

        let fixture = Fixture::new("unbounded-id");
        let encoded_settings = format!(
            "frame-av-settings\nversion=2\nmicrophone=d\nsystem_audio=p;{};a:48000:2:f32\ncamera=d",
            "ab".repeat(64)
        );
        let bytes = serde_json::to_vec(&PersistedSettingsEnvelope {
            schema_version: PERSISTED_SCHEMA_VERSION,
            revision: 2,
            encoded_settings,
        })
        .expect("encode oversized ID envelope");
        write_private(
            &fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
            &bytes,
        );
        assert!(matches!(
            fixture.store.load(),
            Err(DurableAvSettingsError::InvalidSettings)
        ));
    }

    #[test]
    fn read_bound_rejects_oversize_and_truncated_records() {
        let fixture = Fixture::new("exact-bound");
        let mut exact = encode_snapshot(DurableAvSettingsSnapshot {
            revision: 2,
            settings: pinned_settings(1),
        })
        .expect("encode exact-bound fixture");
        exact.resize(AV_SETTINGS_READ_LIMIT_BYTES, b' ');
        write_private(
            &fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
            &exact,
        );
        assert_eq!(
            fixture.store.load().expect("load 4 KiB record").revision(),
            2
        );

        let fixture = Fixture::new("oversize");
        write_private(
            &fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
            &vec![b'x'; AV_SETTINGS_READ_LIMIT_BYTES + 1],
        );
        assert!(matches!(
            fixture.store.load(),
            Err(DurableAvSettingsError::SettingsTooLarge)
        ));

        let fixture = Fixture::new("truncated");
        let encoded = encode_snapshot(DurableAvSettingsSnapshot {
            revision: 2,
            settings: pinned_settings(1),
        })
        .expect("encode truncation fixture");
        write_private(
            &fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
            &encoded[..encoded.len() / 2],
        );
        assert!(matches!(
            fixture.store.load(),
            Err(DurableAvSettingsError::MalformedSettings)
        ));
    }

    #[test]
    fn staged_but_unpublished_write_preserves_last_revision() {
        let mut fixture = Fixture::new("crash-stage");
        let committed = fixture
            .store
            .compare_and_swap(INITIAL_AV_SETTINGS_REVISION, pinned_settings(1))
            .expect("commit first revision");
        let staged_bytes = encode_snapshot(DurableAvSettingsSnapshot {
            revision: 3,
            settings: pinned_settings(2),
        })
        .expect("encode staged revision");
        let _staged = fixture
            .store
            .stage_bytes(SETTINGS_STAGING_NAME, &staged_bytes)
            .expect("stage interrupted write");

        assert_eq!(
            fixture.store.load().expect("load after interruption"),
            committed
        );
        let recovered = fixture
            .store
            .compare_and_swap(committed.revision(), pinned_settings(3))
            .expect("replace abandoned staging file");
        assert_eq!(recovered.revision(), 3);
        assert!(!fixture.settings_path().join(SETTINGS_STAGING_NAME).exists());
    }

    #[test]
    fn symlinks_are_never_followed_or_replaced() {
        let fixture = Fixture::new("symlink");
        let outside = fixture.temporary.path.join("outside");
        write_private(&outside, b"outside must remain unchanged");
        symlink(
            &outside,
            fixture.settings_path().join(SETTINGS_SLOT_NAMES[0]),
        )
        .expect("create malicious settings symlink");

        assert!(matches!(
            fixture.store.load(),
            Err(DurableAvSettingsError::Rooted(_))
        ));
        assert_eq!(
            fs::read(&outside).expect("read outside sentinel"),
            b"outside must remain unchanged"
        );
    }

    #[test]
    fn pinned_directory_survives_visible_path_replacement() {
        let mut fixture = Fixture::new("directory-replacement");
        let revision_two = fixture
            .store
            .compare_and_swap(INITIAL_AV_SETTINGS_REVISION, pinned_settings(1))
            .expect("commit before directory replacement");
        let anchored = fixture.temporary.path.join("anchored-settings");
        fs::rename(fixture.settings_path(), &anchored).expect("rename pinned settings directory");
        fs::create_dir(fixture.settings_path()).expect("create visible replacement directory");
        fs::set_permissions(fixture.settings_path(), fs::Permissions::from_mode(0o700))
            .expect("make replacement directory private");

        let revision_three = fixture
            .store
            .compare_and_swap(revision_two.revision(), pinned_settings(2))
            .expect("commit through pinned descriptor");
        assert_eq!(revision_three.revision(), 3);
        assert!(anchored.join(SETTINGS_SLOT_NAMES[1]).is_file());
        assert!(
            fs::read_dir(fixture.settings_path())
                .expect("read visible replacement")
                .next()
                .is_none()
        );
    }

    #[test]
    fn installation_secret_is_exact_private_stable_and_redacted() {
        let mut fixture = Fixture::new("secret");
        let secret = fixture
            .store
            .create_installation_secret(Zeroizing::new([7; INSTALLATION_SECRET_BYTES]))
            .expect("create installation secret");
        assert_eq!(secret.as_bytes(), &[7; INSTALLATION_SECRET_BYTES]);
        assert_eq!(format!("{secret:?}"), "InstallationSecret(<redacted>)");
        let loaded = fixture
            .store
            .load_installation_secret()
            .expect("load installation secret")
            .expect("secret must exist");
        assert_eq!(loaded.as_bytes(), &[7; INSTALLATION_SECRET_BYTES]);
        let mode = fs::metadata(fixture.settings_path().join(INSTALLATION_SECRET_NAME))
            .expect("secret metadata")
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
    }

    #[test]
    fn installation_secret_rejects_length_zero_value_and_insecure_mode() {
        let fixture = Fixture::new("secret-short");
        write_private(
            &fixture.settings_path().join(INSTALLATION_SECRET_NAME),
            &[1; INSTALLATION_SECRET_BYTES - 1],
        );
        assert!(matches!(
            fixture.store.load_installation_secret(),
            Err(DurableAvSettingsError::InvalidSecretLength)
        ));

        let fixture = Fixture::new("secret-long");
        write_private(
            &fixture.settings_path().join(INSTALLATION_SECRET_NAME),
            &[1; INSTALLATION_SECRET_BYTES + 1],
        );
        assert!(matches!(
            fixture.store.load_installation_secret(),
            Err(DurableAvSettingsError::InvalidSecretLength)
        ));

        let fixture = Fixture::new("secret-zero");
        write_private(
            &fixture.settings_path().join(INSTALLATION_SECRET_NAME),
            &[0; INSTALLATION_SECRET_BYTES],
        );
        assert!(matches!(
            fixture.store.load_installation_secret(),
            Err(DurableAvSettingsError::InvalidInstallationSecret)
        ));

        let fixture = Fixture::new("secret-mode");
        let path = fixture.settings_path().join(INSTALLATION_SECRET_NAME);
        write_private(&path, &[1; INSTALLATION_SECRET_BYTES]);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("make secret mode insecure");
        assert!(matches!(
            fixture.store.load_installation_secret(),
            Err(DurableAvSettingsError::InsecureFileMode)
        ));

        let mut fixture = Fixture::new("secret-generated-zero");
        assert!(matches!(
            fixture
                .store
                .create_installation_secret(Zeroizing::new([0; INSTALLATION_SECRET_BYTES])),
            Err(DurableAvSettingsError::InvalidInstallationSecret)
        ));
    }
}
