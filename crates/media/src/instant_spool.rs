//! OS-keyed, encrypted Instant spool implementation.
//!
//! Payload bytes are encrypted and authenticated before they reach disk. A
//! reservation writes independent bounded AES-256-GCM records, appends an
//! authenticated descriptor only after the complete plaintext hash matches,
//! fsyncs the file, atomically renames it under a per-segment lock, and fsyncs
//! the session directory. Recovery authenticates every record and reconstructs
//! the exact contract descriptor before returning a durable receipt.

#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    InstantCodec, InstantContainer, InstantError, InstantOperationId, InstantSegmentDescriptor,
    InstantSegmentPayload, InstantSessionId, InstantTrackMetadata, InstantTrackRole,
    PrivateSpoolPort, RecoveredSpoolEntry, RuntimeSpoolKeyHandle, Sha256Digest, SpoolAeadAlgorithm,
    SpoolCommitReceipt, SpoolProtectionCapability, SpoolReservationLease, SpoolWriteClaim,
    strong_sha256,
};

const SPOOL_MAGIC: &[u8; 8] = b"FRMSPL02";
const SPOOL_VERSION: u16 = 2;
const HEADER_BYTES: u64 = 8 + 2 + 16 + 16 + 4 + 32 + 8;
#[cfg(test)]
const FRAME_HEADER_BYTES: u64 = 1 + 4 + 4 + 4 + 12;
const FRAME_TAG_CHUNK: u8 = 1;
const FRAME_TAG_DESCRIPTOR: u8 = 2;
const MAX_DESCRIPTOR_BYTES: usize = 4 * 1024;
const MAX_ENCRYPTED_CHUNK_BYTES: usize = crate::MAX_INSTANT_PAYLOAD_CHUNK_BYTES + 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncryptedSpoolPolicy {
    pub max_total_bytes: u64,
    pub max_segment_bytes: u64,
    pub max_files: usize,
}

impl EncryptedSpoolPolicy {
    pub fn validate(self) -> Result<Self, InstantError> {
        if self.max_total_bytes == 0
            || self.max_segment_bytes == 0
            || self.max_segment_bytes > crate::MAX_INSTANT_SEGMENT_BYTES
            || self.max_segment_bytes > self.max_total_bytes
            || self.max_files == 0
            || self.max_files > crate::MAX_INSTANT_SEGMENTS as usize
        {
            return Err(InstantError::InvalidSpoolQuota);
        }
        Ok(self)
    }
}

/// Narrow credential-manager boundary. Implementations must never serialize a
/// key alongside spool files or expose it through diagnostics.
pub trait InstantSpoolKeyStore: fmt::Debug + Send {
    fn load(&mut self, account: &str) -> Result<Option<Zeroizing<[u8; 32]>>, InstantError>;
    fn store(&mut self, account: &str, key: &[u8; 32]) -> Result<(), InstantError>;
    fn delete(&mut self, account: &str) -> Result<(), InstantError>;
}

/// Production OS credential adapter. macOS uses the Security framework,
/// Windows uses the per-user Windows Credential Manager, and Linux uses
/// `secret-tool` with the secret on stdin, never argv.
pub struct OsCredentialSpoolKeyStore {
    service: String,
}

impl fmt::Debug for OsCredentialSpoolKeyStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OsCredentialSpoolKeyStore")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl OsCredentialSpoolKeyStore {
    pub fn new(service: impl Into<String>) -> Result<Self, InstantError> {
        let service = service.into();
        if service.is_empty()
            || service.len() > 128
            || !service
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(InstantError::SpoolKeyUnavailable);
        }
        Ok(Self { service })
    }
}

#[cfg(target_os = "macos")]
impl InstantSpoolKeyStore for OsCredentialSpoolKeyStore {
    fn load(&mut self, account: &str) -> Result<Option<Zeroizing<[u8; 32]>>, InstantError> {
        use security_framework::passwords::get_generic_password;
        match get_generic_password(&self.service, account) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                decode_key_bytes(bytes.as_slice()).map(Some)
            }
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(InstantError::SpoolKeyUnavailable),
        }
    }

    fn store(&mut self, account: &str, key: &[u8; 32]) -> Result<(), InstantError> {
        use security_framework::passwords::set_generic_password;
        set_generic_password(&self.service, account, key)
            .map_err(|_| InstantError::SpoolKeyUnavailable)
    }

    fn delete(&mut self, account: &str) -> Result<(), InstantError> {
        use security_framework::passwords::delete_generic_password;
        match delete_generic_password(&self.service, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err(InstantError::SpoolKeyUnavailable),
        }
    }
}

#[cfg(target_os = "linux")]
impl InstantSpoolKeyStore for OsCredentialSpoolKeyStore {
    fn load(&mut self, account: &str) -> Result<Option<Zeroizing<[u8; 32]>>, InstantError> {
        let tool = secret_tool_path()?;
        let output = Command::new(tool)
            .args([
                "lookup",
                "service",
                self.service.as_str(),
                "account",
                account,
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| InstantError::SpoolKeyUnavailable)?;
        if !output.status.success() {
            return Ok(None);
        }
        let stdout = Zeroizing::new(output.stdout);
        if stdout.len() > 128 {
            return Err(InstantError::SpoolKeyUnavailable);
        }
        decode_hex_key(stdout.as_slice()).map(Some)
    }

    fn store(&mut self, account: &str, key: &[u8; 32]) -> Result<(), InstantError> {
        let tool = secret_tool_path()?;
        let mut child = Command::new(tool)
            .args([
                "store",
                "--label=Frame Instant encrypted spool",
                "service",
                self.service.as_str(),
                "account",
                account,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| InstantError::SpoolKeyUnavailable)?;
        let mut encoded = hex_bytes(key);
        encoded.push('\n');
        child
            .stdin
            .take()
            .ok_or(InstantError::SpoolKeyUnavailable)?
            .write_all(encoded.as_bytes())
            .map_err(|_| InstantError::SpoolKeyUnavailable)?;
        child
            .wait()
            .map_err(|_| InstantError::SpoolKeyUnavailable)?
            .success()
            .then_some(())
            .ok_or(InstantError::SpoolKeyUnavailable)
    }

    fn delete(&mut self, account: &str) -> Result<(), InstantError> {
        let tool = secret_tool_path()?;
        Command::new(tool)
            .args([
                "clear",
                "service",
                self.service.as_str(),
                "account",
                account,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| InstantError::SpoolKeyUnavailable)?
            .success()
            .then_some(())
            .ok_or(InstantError::SpoolKeyUnavailable)
    }
}

#[cfg(windows)]
impl InstantSpoolKeyStore for OsCredentialSpoolKeyStore {
    fn load(&mut self, account: &str) -> Result<Option<Zeroizing<[u8; 32]>>, InstantError> {
        frame_windows_secure_spool::credential_load(&self.service, account)
            .map_err(|_| InstantError::SpoolKeyUnavailable)
    }

    fn store(&mut self, account: &str, key: &[u8; 32]) -> Result<(), InstantError> {
        frame_windows_secure_spool::credential_store(&self.service, account, key)
            .map_err(|_| InstantError::SpoolKeyUnavailable)
    }

    fn delete(&mut self, account: &str) -> Result<(), InstantError> {
        frame_windows_secure_spool::credential_delete(&self.service, account)
            .map_err(|_| InstantError::SpoolKeyUnavailable)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
impl InstantSpoolKeyStore for OsCredentialSpoolKeyStore {
    fn load(&mut self, _account: &str) -> Result<Option<Zeroizing<[u8; 32]>>, InstantError> {
        Err(InstantError::SpoolKeyUnavailable)
    }

    fn store(&mut self, _account: &str, _key: &[u8; 32]) -> Result<(), InstantError> {
        Err(InstantError::SpoolKeyUnavailable)
    }

    fn delete(&mut self, _account: &str) -> Result<(), InstantError> {
        Err(InstantError::SpoolKeyUnavailable)
    }
}

#[cfg(target_os = "linux")]
fn secret_tool_path() -> Result<&'static Path, InstantError> {
    [
        Path::new("/usr/bin/secret-tool"),
        Path::new("/bin/secret-tool"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or(InstantError::SpoolKeyUnavailable)
}

fn decode_key_bytes(bytes: &[u8]) -> Result<Zeroizing<[u8; 32]>, InstantError> {
    if bytes.len() != 32 {
        return Err(InstantError::SpoolKeyUnavailable);
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    key.copy_from_slice(bytes);
    if key.iter().all(|byte| *byte == 0) {
        return Err(InstantError::SpoolKeyUnavailable);
    }
    Ok(key)
}

#[cfg(target_os = "linux")]
fn decode_hex_key(bytes: &[u8]) -> Result<Zeroizing<[u8; 32]>, InstantError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| InstantError::SpoolKeyUnavailable)?
        .trim();
    if text.len() != 64 {
        return Err(InstantError::SpoolKeyUnavailable);
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    for (index, output) in key.iter_mut().enumerate() {
        *output = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| InstantError::SpoolKeyUnavailable)?;
    }
    if key.iter().all(|byte| *byte == 0) {
        return Err(InstantError::SpoolKeyUnavailable);
    }
    Ok(key)
}

#[cfg(target_os = "linux")]
fn hex_bytes(bytes: &[u8]) -> Zeroizing<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(*byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    Zeroizing::new(encoded)
}

/// Concrete `PrivateSpoolPort` used by native desktop Instant recording.
pub struct FilesystemEncryptedSpool<K: InstantSpoolKeyStore> {
    root: PathBuf,
    policy: EncryptedSpoolPolicy,
    key_store: K,
    active_keys: BTreeMap<[u8; 16], Zeroizing<[u8; 32]>>,
}

impl<K: InstantSpoolKeyStore> fmt::Debug for FilesystemEncryptedSpool<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemEncryptedSpool")
            .field("policy", &self.policy)
            .field("active_key_count", &self.active_keys.len())
            .finish_non_exhaustive()
    }
}

impl FilesystemEncryptedSpool<OsCredentialSpoolKeyStore> {
    pub fn with_os_key_store(
        root: impl Into<PathBuf>,
        policy: EncryptedSpoolPolicy,
    ) -> Result<Self, InstantError> {
        Self::new(
            root,
            policy,
            OsCredentialSpoolKeyStore::new("xyz.engmanager.frame.instant-spool")?,
        )
    }
}

impl<K: InstantSpoolKeyStore> FilesystemEncryptedSpool<K> {
    pub fn new(
        root: impl Into<PathBuf>,
        policy: EncryptedSpoolPolicy,
        key_store: K,
    ) -> Result<Self, InstantError> {
        let root = root.into();
        let policy = policy.validate()?;
        create_private_directory(&root)?;
        Ok(Self {
            root,
            policy,
            key_store,
            active_keys: BTreeMap::new(),
        })
    }

    fn key(&self, handle: &RuntimeSpoolKeyHandle) -> Result<Zeroizing<[u8; 32]>, InstantError> {
        self.active_keys
            .get(&handle.canonical_bytes())
            .cloned()
            .ok_or(InstantError::SpoolKeyUnavailable)
    }

    fn session_directory(&self, session_id: InstantSessionId) -> PathBuf {
        self.root.join(hex(&session_id.canonical_bytes()))
    }

    fn account(session_id: InstantSessionId) -> String {
        hex(&session_id.canonical_bytes())
    }

    fn current_declared_bytes(&self) -> Result<(u64, usize), InstantError> {
        let mut bytes = 0_u64;
        let mut files = 0_usize;
        for session in fs::read_dir(&self.root).map_err(|_| InstantError::SpoolDiskFull)? {
            let session = session.map_err(|_| InstantError::SpoolDiskFull)?;
            let metadata =
                fs::symlink_metadata(session.path()).map_err(|_| InstantError::SpoolDiskFull)?;
            if metadata_is_indirect(&metadata) || !metadata.is_dir() {
                return Err(InstantError::SpoolCorrupt);
            }
            validate_private_directory(&session.path())?;
            for entry in fs::read_dir(session.path()).map_err(|_| InstantError::SpoolDiskFull)? {
                let entry = entry.map_err(|_| InstantError::SpoolDiskFull)?;
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or(InstantError::SpoolCorrupt)?;
                if !(name.ends_with(".spool") || name.ends_with(".tmp")) {
                    continue;
                }
                let metadata =
                    fs::symlink_metadata(&path).map_err(|_| InstantError::SpoolDiskFull)?;
                if metadata_is_indirect(&metadata) || !metadata.is_file() {
                    return Err(InstantError::SpoolCorrupt);
                }
                enforce_windows_private_permissions(&path)?;
                let header =
                    read_header(&mut File::open(path).map_err(|_| InstantError::SpoolDiskFull)?)?;
                bytes = bytes
                    .checked_add(header.declared_bytes)
                    .ok_or(InstantError::SpoolQuotaExceeded)?;
                files = files
                    .checked_add(1)
                    .ok_or(InstantError::SpoolQuotaExceeded)?;
            }
        }
        Ok((bytes, files))
    }
}

impl<K: InstantSpoolKeyStore + 'static> PrivateSpoolPort for FilesystemEncryptedSpool<K> {
    fn protection(&self) -> SpoolProtectionCapability {
        SpoolProtectionCapability::EncryptedAndAuthenticated {
            algorithm: SpoolAeadAlgorithm::Aes256Gcm,
            atomic_replace: true,
            private_permissions: true,
        }
    }

    fn acquire_runtime_key(
        &mut self,
        session_id: InstantSessionId,
    ) -> Result<RuntimeSpoolKeyHandle, InstantError> {
        let account = Self::account(session_id);
        let key = match self.key_store.load(&account)? {
            Some(key) => key,
            None => {
                if session_contains_committed_spool(&self.session_directory(session_id))? {
                    return Err(InstantError::SpoolKeyUnavailable);
                }
                let mut generated = Zeroizing::new([0_u8; 32]);
                SystemRandom::new()
                    .fill(generated.as_mut())
                    .map_err(|_| InstantError::SpoolKeyUnavailable)?;
                self.key_store.store(&account, &generated)?;
                generated
            }
        };
        let digest = Sha256::digest(key.as_ref());
        let mut handle = [0_u8; 16];
        handle.copy_from_slice(&digest[..16]);
        if handle.iter().all(|byte| *byte == 0) {
            return Err(InstantError::SpoolKeyUnavailable);
        }
        self.active_keys.insert(handle, key);
        RuntimeSpoolKeyHandle::from_runtime(handle)
    }

    fn key_marker(&self, key: &RuntimeSpoolKeyHandle) -> Sha256Digest {
        strong_sha256(&key.canonical_bytes())
    }

    fn reserve(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        claim: SpoolWriteClaim,
    ) -> Result<Box<dyn SpoolReservationLease>, InstantError> {
        if claim.bytes == 0 || claim.bytes > self.policy.max_segment_bytes {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        let (declared, files) = self.current_declared_bytes()?;
        if files >= self.policy.max_files
            || declared
                .checked_add(claim.bytes)
                .is_none_or(|value| value > self.policy.max_total_bytes)
        {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        let directory = self.session_directory(claim.session_id);
        create_private_directory(&directory)?;
        let temp = directory.join(format!(
            ".{}-{}.tmp",
            claim.segment_index,
            hex(&claim.operation_id.canonical_bytes())
        ));
        let final_path = directory.join(final_name(claim.segment_index, claim.segment_identity));
        let lock_path = final_path.with_extension("lock");
        let file = private_create_new(&temp)?;
        let mut reservation = EncryptedSpoolReservation {
            file,
            temp_path: temp,
            final_path,
            lock_path,
            session_directory: directory,
            claim,
            key: self.key(key)?,
            written: 0,
            sequence: 0,
            plaintext_hash: Sha256::new(),
            terminal: false,
        };
        reservation.write_header()?;
        Ok(Box::new(reservation))
    }

    fn open(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<Box<dyn InstantSegmentPayload>, InstantError> {
        let path = self
            .session_directory(session_id)
            .join(final_name(descriptor.index(), descriptor.identity()));
        let active_key = self.key(key)?;
        let inspected = inspect_file(&path, &active_key)?;
        if inspected.descriptor != *descriptor {
            return Err(InstantError::SpoolCorrupt);
        }
        Ok(Box::new(EncryptedSpoolPayload {
            file: File::open(path).map_err(|_| InstantError::SpoolEntryMissing)?,
            key: self.key(key)?,
            header: inspected.header,
            frames: inspected.frames,
            current: 0,
            pending: Vec::new(),
            pending_offset: 0,
            declared_len: descriptor.bytes(),
            terminal: false,
        }))
    }

    fn recover(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
    ) -> Result<Vec<RecoveredSpoolEntry>, InstantError> {
        let directory = self.session_directory(session_id);
        if !directory.exists() {
            return Ok(Vec::new());
        }
        validate_private_directory(&directory)?;
        let key = self.key(key)?;
        let mut paths = Vec::new();
        for entry in fs::read_dir(&directory).map_err(|_| InstantError::SpoolCorrupt)? {
            let path = entry.map_err(|_| InstantError::SpoolCorrupt)?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| InstantError::SpoolCorrupt)?;
            if metadata_is_indirect(&metadata) || !metadata.is_file() {
                return Err(InstantError::SpoolCorrupt);
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or(InstantError::SpoolCorrupt)?;
            if name.ends_with(".tmp") || name.ends_with(".lock") {
                fs::remove_file(&path).map_err(|_| InstantError::SpoolDiskFull)?;
            } else if name.ends_with(".spool") {
                paths.push(path);
            } else {
                return Err(InstantError::SpoolCorrupt);
            }
        }
        sync_directory(&directory)?;
        paths.sort();
        if paths.len() > self.policy.max_files {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        let mut entries = Vec::with_capacity(paths.len());
        let mut total = 0_u64;
        for path in paths {
            let inspected = inspect_file(&path, &key)?;
            if inspected.header.session_id != session_id {
                return Err(InstantError::SpoolCorrupt);
            }
            total = total
                .checked_add(inspected.descriptor.bytes())
                .ok_or(InstantError::SpoolQuotaExceeded)?;
            entries.push(RecoveredSpoolEntry {
                descriptor: inspected.descriptor,
                commit_receipt: inspected.receipt,
                committed: true,
            });
        }
        if total > self.policy.max_total_bytes {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        entries.sort_by_key(|entry| entry.descriptor.index());
        Ok(entries)
    }

    fn evict(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
        segment_identity: Sha256Digest,
    ) -> Result<(), InstantError> {
        let directory = self.session_directory(session_id);
        let key = self.key(key)?;
        let mut found = None;
        for entry in fs::read_dir(&directory).map_err(|_| InstantError::SpoolEntryMissing)? {
            let path = entry.map_err(|_| InstantError::SpoolCorrupt)?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("spool") {
                let inspected = inspect_file(&path, &key)?;
                if inspected.descriptor.identity() == segment_identity
                    && found.replace(path).is_some()
                {
                    return Err(InstantError::SpoolCorrupt);
                }
            }
        }
        fs::remove_file(found.ok_or(InstantError::SpoolEntryMissing)?)
            .map_err(|_| InstantError::SpoolDiskFull)?;
        sync_directory(&directory)
    }

    fn wipe_session(
        &mut self,
        key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
    ) -> Result<(), InstantError> {
        let directory = self.session_directory(session_id);
        if directory.exists() {
            validate_private_directory(&directory)?;
            for entry in fs::read_dir(&directory).map_err(|_| InstantError::SpoolDiskFull)? {
                let path = entry.map_err(|_| InstantError::SpoolDiskFull)?.path();
                let metadata =
                    fs::symlink_metadata(&path).map_err(|_| InstantError::SpoolCorrupt)?;
                if metadata_is_indirect(&metadata) || !metadata.is_file() {
                    return Err(InstantError::SpoolCorrupt);
                }
                fs::remove_file(path).map_err(|_| InstantError::SpoolDiskFull)?;
            }
            sync_directory(&directory)?;
            fs::remove_dir(&directory).map_err(|_| InstantError::SpoolDiskFull)?;
            sync_directory(&self.root)?;
        }
        self.key_store.delete(&Self::account(session_id))?;
        if let Some(mut removed) = self.active_keys.remove(&key.canonical_bytes()) {
            removed.zeroize();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct SpoolHeader {
    session_id: InstantSessionId,
    operation_id: InstantOperationId,
    segment_index: u32,
    segment_identity: Sha256Digest,
    declared_bytes: u64,
}

struct EncryptedSpoolReservation {
    file: File,
    temp_path: PathBuf,
    final_path: PathBuf,
    lock_path: PathBuf,
    session_directory: PathBuf,
    claim: SpoolWriteClaim,
    key: Zeroizing<[u8; 32]>,
    written: u64,
    sequence: u32,
    plaintext_hash: Sha256,
    terminal: bool,
}

impl fmt::Debug for EncryptedSpoolReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncryptedSpoolReservation")
            .field("segment_index", &self.claim.segment_index)
            .field("written", &self.written)
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl EncryptedSpoolReservation {
    fn header(&self) -> SpoolHeader {
        SpoolHeader {
            session_id: self.claim.session_id,
            operation_id: self.claim.operation_id,
            segment_index: self.claim.segment_index,
            segment_identity: self.claim.segment_identity,
            declared_bytes: self.claim.bytes,
        }
    }

    fn write_header(&mut self) -> Result<(), InstantError> {
        let header = self.header();
        write_header(&mut self.file, header)
    }

    fn append_encrypted(
        &mut self,
        tag: u8,
        sequence: u32,
        bytes: &[u8],
    ) -> Result<(), InstantError> {
        let plain_len = u32::try_from(bytes.len()).map_err(|_| InstantError::SpoolDiskFull)?;
        let mut nonce_bytes = [0_u8; 12];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| InstantError::SpoolDiskFull)?;
        let mut encrypted = Zeroizing::new(bytes.to_vec());
        encryption_key(&self.key)?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(aad(self.header(), tag, sequence, plain_len)),
                &mut *encrypted,
            )
            .map_err(|_| InstantError::SpoolDiskFull)?;
        let encrypted_len =
            u32::try_from(encrypted.len()).map_err(|_| InstantError::SpoolDiskFull)?;
        self.file
            .write_all(&[tag])
            .and_then(|()| self.file.write_all(&sequence.to_be_bytes()))
            .and_then(|()| self.file.write_all(&plain_len.to_be_bytes()))
            .and_then(|()| self.file.write_all(&encrypted_len.to_be_bytes()))
            .and_then(|()| self.file.write_all(&nonce_bytes))
            .and_then(|()| self.file.write_all(&encrypted))
            .map_err(|_| InstantError::SpoolDiskFull)
    }

    fn cleanup(&mut self) {
        let _ = fs::remove_file(&self.temp_path);
        let _ = fs::remove_file(&self.lock_path);
    }
}

impl SpoolReservationLease for EncryptedSpoolReservation {
    fn write(&mut self, chunk: &[u8]) -> Result<(), InstantError> {
        if self.terminal
            || chunk.is_empty()
            || chunk.len() > crate::MAX_INSTANT_PAYLOAD_CHUNK_BYTES
            || self
                .written
                .checked_add(chunk.len() as u64)
                .is_none_or(|value| value > self.claim.bytes)
        {
            return Err(InstantError::SpoolDiskFull);
        }
        self.append_encrypted(FRAME_TAG_CHUNK, self.sequence, chunk)?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(InstantError::SpoolDiskFull)?;
        self.written += chunk.len() as u64;
        self.plaintext_hash.update(chunk);
        Ok(())
    }

    fn commit(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<SpoolCommitReceipt, InstantError> {
        if self.terminal
            || self.written != self.claim.bytes
            || descriptor.session_id() != self.claim.session_id
            || descriptor.index() != self.claim.segment_index
            || descriptor.identity() != self.claim.segment_identity
            || descriptor.bytes() != self.claim.bytes
            || self.plaintext_hash.clone().finalize().as_slice()
                != descriptor.sha256().canonical_bytes()
        {
            return Err(InstantError::InvalidSpoolReceipt);
        }
        let encoded = Zeroizing::new(encode_descriptor(descriptor)?);
        self.append_encrypted(FRAME_TAG_DESCRIPTOR, u32::MAX, encoded.as_slice())?;
        self.file
            .sync_all()
            .map_err(|_| InstantError::SpoolDiskFull)?;
        let _lock = private_create_new(&self.lock_path)?;
        #[cfg(not(windows))]
        if self.final_path.exists() {
            self.cleanup();
            self.terminal = true;
            return Err(InstantError::OperationAlreadyApplied);
        }
        publish_spool_file(&self.temp_path, &self.final_path)?;
        sync_directory(&self.session_directory)?;
        let integrity = file_digest(&self.final_path)?;
        fs::remove_file(&self.lock_path).map_err(|_| InstantError::SpoolDiskFull)?;
        sync_directory(&self.session_directory)?;
        self.terminal = true;
        Ok(SpoolCommitReceipt {
            segment_index: descriptor.index(),
            segment_identity: descriptor.identity(),
            bytes: descriptor.bytes(),
            ciphertext_integrity: integrity,
            durable: true,
        })
    }

    fn abort(&mut self) {
        self.cleanup();
        self.terminal = true;
    }
}

impl Drop for EncryptedSpoolReservation {
    fn drop(&mut self) {
        if !self.terminal {
            self.cleanup();
        }
    }
}

#[derive(Debug, Clone)]
struct EncryptedFrameIndex {
    sequence: u32,
    plain_len: u32,
    encrypted_len: u32,
    nonce: [u8; 12],
    offset: u64,
}

struct InspectedSpool {
    header: SpoolHeader,
    descriptor: InstantSegmentDescriptor,
    receipt: SpoolCommitReceipt,
    frames: Vec<EncryptedFrameIndex>,
}

struct EncryptedSpoolPayload {
    file: File,
    key: Zeroizing<[u8; 32]>,
    header: SpoolHeader,
    frames: Vec<EncryptedFrameIndex>,
    current: usize,
    pending: Vec<u8>,
    pending_offset: usize,
    declared_len: u64,
    terminal: bool,
}

impl fmt::Debug for EncryptedSpoolPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncryptedSpoolPayload")
            .field("declared_len", &self.declared_len)
            .field("frame_count", &self.frames.len())
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl InstantSegmentPayload for EncryptedSpoolPayload {
    fn declared_len(&self) -> u64 {
        self.declared_len
    }

    fn pull(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, InstantError> {
        if self.terminal {
            return Ok(None);
        }
        if max_bytes == 0 || max_bytes > crate::MAX_INSTANT_PAYLOAD_CHUNK_BYTES {
            return Err(InstantError::InvalidPayloadChunk);
        }
        if self.pending_offset == self.pending.len() {
            self.pending.clear();
            self.pending_offset = 0;
            let Some(frame) = self.frames.get(self.current).cloned() else {
                self.terminal = true;
                return Ok(None);
            };
            self.current += 1;
            self.file
                .seek(SeekFrom::Start(frame.offset))
                .map_err(|_| InstantError::SpoolCorrupt)?;
            let mut encrypted = Zeroizing::new(vec![0_u8; frame.encrypted_len as usize]);
            self.file
                .read_exact(encrypted.as_mut_slice())
                .map_err(|_| InstantError::SpoolCorrupt)?;
            let opened = encryption_key(&self.key)?
                .open_in_place(
                    Nonce::assume_unique_for_key(frame.nonce),
                    Aad::from(aad(
                        self.header,
                        FRAME_TAG_CHUNK,
                        frame.sequence,
                        frame.plain_len,
                    )),
                    encrypted.as_mut_slice(),
                )
                .map_err(|_| InstantError::SpoolCorrupt)?;
            if opened.len() != frame.plain_len as usize {
                return Err(InstantError::SpoolCorrupt);
            }
            self.pending.extend_from_slice(opened);
        }
        let end = self
            .pending_offset
            .saturating_add(max_bytes)
            .min(self.pending.len());
        let chunk = self.pending[self.pending_offset..end].to_vec();
        self.pending_offset = end;
        if chunk.is_empty() {
            return Err(InstantError::SpoolCorrupt);
        }
        Ok(Some(chunk))
    }

    fn cancel(&mut self) {
        self.pending.zeroize();
        self.terminal = true;
    }
}

impl Drop for EncryptedSpoolPayload {
    fn drop(&mut self) {
        self.pending.zeroize();
    }
}

fn inspect_file(path: &Path, key: &[u8; 32]) -> Result<InspectedSpool, InstantError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| InstantError::SpoolEntryMissing)?;
    if metadata_is_indirect(&metadata) || !metadata.is_file() || metadata.len() <= HEADER_BYTES {
        return Err(InstantError::SpoolCorrupt);
    }
    enforce_windows_private_permissions(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(InstantError::SpoolCorrupt);
        }
    }
    let mut file = File::open(path).map_err(|_| InstantError::SpoolCorrupt)?;
    let header = read_header(&mut file)?;
    let mut frames = Vec::new();
    let mut plaintext_hash = Sha256::new();
    let mut plaintext_bytes = 0_u64;
    let mut descriptor = None;
    loop {
        let mut tag = [0_u8; 1];
        let read = file
            .read(&mut tag)
            .map_err(|_| InstantError::SpoolCorrupt)?;
        if read == 0 {
            break;
        }
        let sequence = read_u32(&mut file)?;
        let plain_len = read_u32(&mut file)?;
        let encrypted_len = read_u32(&mut file)?;
        let mut nonce = [0_u8; 12];
        file.read_exact(&mut nonce)
            .map_err(|_| InstantError::SpoolCorrupt)?;
        if encrypted_len
            != plain_len
                .checked_add(16)
                .ok_or(InstantError::SpoolCorrupt)?
            || encrypted_len as usize > MAX_ENCRYPTED_CHUNK_BYTES.max(MAX_DESCRIPTOR_BYTES + 16)
        {
            return Err(InstantError::SpoolCorrupt);
        }
        let offset = file
            .stream_position()
            .map_err(|_| InstantError::SpoolCorrupt)?;
        let mut encrypted = Zeroizing::new(vec![0_u8; encrypted_len as usize]);
        file.read_exact(encrypted.as_mut_slice())
            .map_err(|_| InstantError::SpoolCorrupt)?;
        let opened = encryption_key(key)?
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad(header, tag[0], sequence, plain_len)),
                encrypted.as_mut_slice(),
            )
            .map_err(|_| InstantError::SpoolCorrupt)?;
        if opened.len() != plain_len as usize {
            return Err(InstantError::SpoolCorrupt);
        }
        match tag[0] {
            FRAME_TAG_CHUNK if descriptor.is_none() && sequence == frames.len() as u32 => {
                if plain_len == 0 || plain_len as usize > crate::MAX_INSTANT_PAYLOAD_CHUNK_BYTES {
                    return Err(InstantError::SpoolCorrupt);
                }
                plaintext_bytes = plaintext_bytes
                    .checked_add(u64::from(plain_len))
                    .ok_or(InstantError::SpoolCorrupt)?;
                plaintext_hash.update(opened);
                frames.push(EncryptedFrameIndex {
                    sequence,
                    plain_len,
                    encrypted_len,
                    nonce,
                    offset,
                });
            }
            FRAME_TAG_DESCRIPTOR if descriptor.is_none() && sequence == u32::MAX => {
                descriptor = Some(decode_descriptor(opened)?);
            }
            _ => return Err(InstantError::SpoolCorrupt),
        }
    }
    let descriptor = descriptor.ok_or(InstantError::SpoolCorrupt)?;
    if plaintext_bytes != header.declared_bytes
        || descriptor.session_id() != header.session_id
        || descriptor.index() != header.segment_index
        || descriptor.identity() != header.segment_identity
        || descriptor.bytes() != header.declared_bytes
        || plaintext_hash.finalize().as_slice() != descriptor.sha256().canonical_bytes()
    {
        return Err(InstantError::SpoolCorrupt);
    }
    Ok(InspectedSpool {
        header,
        receipt: SpoolCommitReceipt {
            segment_index: descriptor.index(),
            segment_identity: descriptor.identity(),
            bytes: descriptor.bytes(),
            ciphertext_integrity: file_digest(path)?,
            durable: true,
        },
        descriptor,
        frames,
    })
}

fn encryption_key(key: &[u8; 32]) -> Result<LessSafeKey, InstantError> {
    UnboundKey::new(&aead::AES_256_GCM, key)
        .map(LessSafeKey::new)
        .map_err(|_| InstantError::SpoolKeyUnavailable)
}

fn aad(header: SpoolHeader, tag: u8, sequence: u32, plain_len: u32) -> Vec<u8> {
    let mut value = Vec::with_capacity(96);
    value.extend_from_slice(SPOOL_MAGIC);
    value.extend_from_slice(&SPOOL_VERSION.to_be_bytes());
    value.extend_from_slice(&header.session_id.canonical_bytes());
    value.extend_from_slice(&header.operation_id.canonical_bytes());
    value.extend_from_slice(&header.segment_index.to_be_bytes());
    value.extend_from_slice(&header.segment_identity.canonical_bytes());
    value.extend_from_slice(&header.declared_bytes.to_be_bytes());
    value.push(tag);
    value.extend_from_slice(&sequence.to_be_bytes());
    value.extend_from_slice(&plain_len.to_be_bytes());
    value
}

fn write_header(file: &mut File, header: SpoolHeader) -> Result<(), InstantError> {
    file.write_all(SPOOL_MAGIC)
        .and_then(|()| file.write_all(&SPOOL_VERSION.to_be_bytes()))
        .and_then(|()| file.write_all(&header.session_id.canonical_bytes()))
        .and_then(|()| file.write_all(&header.operation_id.canonical_bytes()))
        .and_then(|()| file.write_all(&header.segment_index.to_be_bytes()))
        .and_then(|()| file.write_all(&header.segment_identity.canonical_bytes()))
        .and_then(|()| file.write_all(&header.declared_bytes.to_be_bytes()))
        .map_err(|_| InstantError::SpoolDiskFull)
}

fn read_header(file: &mut File) -> Result<SpoolHeader, InstantError> {
    let mut magic = [0_u8; 8];
    file.read_exact(&mut magic)
        .map_err(|_| InstantError::SpoolCorrupt)?;
    if &magic != SPOOL_MAGIC || read_u16(file)? != SPOOL_VERSION {
        return Err(InstantError::SpoolCorrupt);
    }
    let mut session = [0_u8; 16];
    let mut operation = [0_u8; 16];
    let mut identity = [0_u8; 32];
    file.read_exact(&mut session)
        .and_then(|()| file.read_exact(&mut operation))
        .map_err(|_| InstantError::SpoolCorrupt)?;
    let segment_index = read_u32(file)?;
    file.read_exact(&mut identity)
        .map_err(|_| InstantError::SpoolCorrupt)?;
    let declared_bytes = read_u64(file)?;
    if declared_bytes == 0 || declared_bytes > crate::MAX_INSTANT_SEGMENT_BYTES {
        return Err(InstantError::SpoolCorrupt);
    }
    Ok(SpoolHeader {
        session_id: InstantSessionId::from_csprng(session)?,
        operation_id: InstantOperationId::from_csprng(operation)?,
        segment_index,
        segment_identity: Sha256Digest::from_bytes(identity)?,
        declared_bytes,
    })
}

fn encode_descriptor(descriptor: &InstantSegmentDescriptor) -> Result<Vec<u8>, InstantError> {
    let mut bytes = Vec::with_capacity(256);
    bytes.extend_from_slice(&crate::INSTANT_PROTOCOL_VERSION.to_be_bytes());
    bytes.extend_from_slice(&descriptor.session_id().canonical_bytes());
    bytes.extend_from_slice(&descriptor.index().to_be_bytes());
    bytes.extend_from_slice(&descriptor.start_ns().to_be_bytes());
    bytes.extend_from_slice(&descriptor.duration_ns().to_be_bytes());
    bytes.push(u8::from(descriptor.starts_with_video_keyframe()));
    bytes.push(match descriptor.container() {
        InstantContainer::FragmentedMp4Cmaf => 1,
    });
    bytes.extend_from_slice(&descriptor.bytes().to_be_bytes());
    bytes.extend_from_slice(&descriptor.sha256().canonical_bytes());
    bytes.extend_from_slice(
        &u16::try_from(descriptor.tracks().len())
            .map_err(|_| InstantError::SpoolCorrupt)?
            .to_be_bytes(),
    );
    for track in descriptor.tracks() {
        bytes.extend_from_slice(&track.track_number().to_be_bytes());
        bytes.push(match track.role() {
            InstantTrackRole::ScreenVideo => 1,
            InstantTrackRole::CameraVideo => 2,
            InstantTrackRole::MixedAudio => 3,
        });
        bytes.push(match track.codec() {
            InstantCodec::H264Avc => 1,
            InstantCodec::AacLowComplexity => 2,
        });
        bytes.extend_from_slice(&track.timescale().to_be_bytes());
        bytes.extend_from_slice(&track.sample_count().to_be_bytes());
        bytes.extend_from_slice(&track.first_presentation_ns().to_be_bytes());
        bytes.extend_from_slice(&track.duration_ns().to_be_bytes());
    }
    if bytes.len() > MAX_DESCRIPTOR_BYTES {
        return Err(InstantError::SpoolCorrupt);
    }
    Ok(bytes)
}

fn decode_descriptor(bytes: &[u8]) -> Result<InstantSegmentDescriptor, InstantError> {
    let mut reader = SliceReader::new(bytes);
    if reader.u16()? != crate::INSTANT_PROTOCOL_VERSION {
        return Err(InstantError::SpoolCorrupt);
    }
    let session_id = InstantSessionId::from_csprng(reader.array()?)?;
    let index = reader.u32()?;
    let start_ns = reader.u64()?;
    let duration_ns = reader.u64()?;
    let keyframe = match reader.u8()? {
        1 => true,
        _ => return Err(InstantError::SpoolCorrupt),
    };
    let container = match reader.u8()? {
        1 => InstantContainer::FragmentedMp4Cmaf,
        _ => return Err(InstantError::SpoolCorrupt),
    };
    let declared_bytes = reader.u64()?;
    let checksum = Sha256Digest::from_bytes(reader.array()?)?;
    let track_count = usize::from(reader.u16()?);
    if track_count == 0 || track_count > crate::MAX_INSTANT_TRACKS {
        return Err(InstantError::SpoolCorrupt);
    }
    let mut tracks = Vec::with_capacity(track_count);
    for _ in 0..track_count {
        let track_number = reader.u16()?;
        let role = match reader.u8()? {
            1 => InstantTrackRole::ScreenVideo,
            2 => InstantTrackRole::CameraVideo,
            3 => InstantTrackRole::MixedAudio,
            _ => return Err(InstantError::SpoolCorrupt),
        };
        let codec = match reader.u8()? {
            1 => InstantCodec::H264Avc,
            2 => InstantCodec::AacLowComplexity,
            _ => return Err(InstantError::SpoolCorrupt),
        };
        tracks.push(InstantTrackMetadata::new(
            track_number,
            role,
            codec,
            reader.u32()?,
            reader.u32()?,
            reader.u64()?,
            reader.u64()?,
        )?);
    }
    if !reader.exhausted() {
        return Err(InstantError::SpoolCorrupt);
    }
    InstantSegmentDescriptor::new(
        session_id,
        index,
        start_ns,
        duration_ns,
        keyframe,
        container,
        tracks,
        declared_bytes,
        checksum,
    )
}

struct SliceReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> SliceReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], InstantError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or(InstantError::SpoolCorrupt)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(InstantError::SpoolCorrupt)?;
        self.offset = end;
        value.try_into().map_err(|_| InstantError::SpoolCorrupt)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], InstantError> {
        self.take()
    }

    fn u8(&mut self) -> Result<u8, InstantError> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, InstantError> {
        Ok(u16::from_be_bytes(self.take()?))
    }

    fn u32(&mut self) -> Result<u32, InstantError> {
        Ok(u32::from_be_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, InstantError> {
        Ok(u64::from_be_bytes(self.take()?))
    }

    fn exhausted(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

fn read_u16(reader: &mut impl Read) -> Result<u16, InstantError> {
    let mut bytes = [0_u8; 2];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| InstantError::SpoolCorrupt)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, InstantError> {
    let mut bytes = [0_u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| InstantError::SpoolCorrupt)?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, InstantError> {
    let mut bytes = [0_u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| InstantError::SpoolCorrupt)?;
    Ok(u64::from_be_bytes(bytes))
}

#[cfg(not(windows))]
fn private_create_new(path: &Path) -> Result<File, InstantError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| InstantError::SpoolDiskFull)
}

#[cfg(windows)]
fn private_create_new(path: &Path) -> Result<File, InstantError> {
    frame_windows_secure_spool::create_private_file(path)
        .map_err(|_| InstantError::SecureSpoolUnavailable)
}

fn create_private_directory(path: &Path) -> Result<(), InstantError> {
    fs::create_dir_all(path).map_err(|_| InstantError::SpoolDiskFull)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| InstantError::SpoolDiskFull)?;
    }
    validate_private_directory(path)
}

fn validate_private_directory(path: &Path) -> Result<(), InstantError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| InstantError::SpoolDiskFull)?;
    if metadata_is_indirect(&metadata) || !metadata.is_dir() {
        return Err(InstantError::SpoolCorrupt);
    }
    enforce_windows_private_permissions(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(InstantError::SecureSpoolUnavailable);
        }
    }
    Ok(())
}

fn session_contains_committed_spool(path: &Path) -> Result<bool, InstantError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(InstantError::SpoolCorrupt),
    };
    if metadata_is_indirect(&metadata) || !metadata.is_dir() {
        return Err(InstantError::SpoolCorrupt);
    }
    validate_private_directory(path)?;
    for entry in fs::read_dir(path).map_err(|_| InstantError::SpoolCorrupt)? {
        let entry = entry.map_err(|_| InstantError::SpoolCorrupt)?;
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| InstantError::SpoolCorrupt)?;
        if metadata_is_indirect(&metadata) || !metadata.is_file() {
            return Err(InstantError::SpoolCorrupt);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| InstantError::SpoolCorrupt)?;
        if name.ends_with(".spool") {
            return Ok(true);
        }
        if !(name.ends_with(".tmp") || name.ends_with(".lock")) {
            return Err(InstantError::SpoolCorrupt);
        }
    }
    Ok(false)
}

#[cfg(not(windows))]
fn metadata_is_indirect(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn metadata_is_indirect(metadata: &fs::Metadata) -> bool {
    frame_windows_secure_spool::metadata_is_indirect(metadata)
}

#[cfg(not(windows))]
fn enforce_windows_private_permissions(_path: &Path) -> Result<(), InstantError> {
    Ok(())
}

#[cfg(windows)]
fn enforce_windows_private_permissions(path: &Path) -> Result<(), InstantError> {
    frame_windows_secure_spool::enforce_private_permissions(path)
        .map_err(|_| InstantError::SecureSpoolUnavailable)
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<(), InstantError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| InstantError::SpoolDiskFull)
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), InstantError> {
    // Windows does not expose a portable directory-fsync equivalent through
    // std. Durable publication is performed by publish_spool_file with
    // MOVEFILE_WRITE_THROUGH. If a later cleanup is interrupted, recovery may
    // retain encrypted garbage but will authenticate or remove it fail closed.
    Ok(())
}

#[cfg(not(windows))]
fn publish_spool_file(source: &Path, destination: &Path) -> Result<(), InstantError> {
    fs::rename(source, destination).map_err(|_| InstantError::SpoolDiskFull)
}

#[cfg(windows)]
fn publish_spool_file(source: &Path, destination: &Path) -> Result<(), InstantError> {
    match frame_windows_secure_spool::publish_file(source, destination) {
        Ok(()) => Ok(()),
        Err(frame_windows_secure_spool::WindowsPublishError::AlreadyExists) => {
            Err(InstantError::OperationAlreadyApplied)
        }
        Err(frame_windows_secure_spool::WindowsPublishError::Failed) => {
            Err(InstantError::SpoolDiskFull)
        }
    }
}

fn final_name(index: u32, identity: Sha256Digest) -> String {
    format!("{index:06}-{}.spool", hex(&identity.canonical_bytes()))
}

fn file_digest(path: &Path) -> Result<Sha256Digest, InstantError> {
    let mut file = File::open(path).map_err(|_| InstantError::SpoolCorrupt)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| InstantError::SpoolCorrupt)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Sha256Digest::from_bytes(digest.finalize().into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::{InstantSpool, SpoolQuotaPolicy};

    #[derive(Debug, Clone, Default)]
    struct TestKeyStore {
        keys: Arc<Mutex<BTreeMap<String, [u8; 32]>>>,
    }

    impl InstantSpoolKeyStore for TestKeyStore {
        fn load(&mut self, account: &str) -> Result<Option<Zeroizing<[u8; 32]>>, InstantError> {
            Ok(self
                .keys
                .lock()
                .map_err(|_| InstantError::SpoolKeyUnavailable)?
                .get(account)
                .copied()
                .map(Zeroizing::new))
        }

        fn store(&mut self, account: &str, key: &[u8; 32]) -> Result<(), InstantError> {
            self.keys
                .lock()
                .map_err(|_| InstantError::SpoolKeyUnavailable)?
                .insert(account.to_owned(), *key);
            Ok(())
        }

        fn delete(&mut self, account: &str) -> Result<(), InstantError> {
            self.keys
                .lock()
                .map_err(|_| InstantError::SpoolKeyUnavailable)?
                .remove(account);
            Ok(())
        }
    }

    #[derive(Debug)]
    struct BytesPayload {
        bytes: Vec<u8>,
        offset: usize,
    }

    impl InstantSegmentPayload for BytesPayload {
        fn declared_len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn pull(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, InstantError> {
            if self.offset == self.bytes.len() {
                return Ok(None);
            }
            let end = self
                .offset
                .saturating_add(max_bytes.min(7))
                .min(self.bytes.len());
            let value = self.bytes[self.offset..end].to_vec();
            self.offset = end;
            Ok(Some(value))
        }
        fn cancel(&mut self) {
            self.bytes.zeroize();
            self.offset = self.bytes.len();
        }
    }

    fn descriptor(session: InstantSessionId, bytes: &[u8]) -> InstantSegmentDescriptor {
        InstantSegmentDescriptor::new(
            session,
            0,
            0,
            1_000_000_000,
            true,
            InstantContainer::FragmentedMp4Cmaf,
            vec![
                InstantTrackMetadata::new(
                    1,
                    InstantTrackRole::ScreenVideo,
                    InstantCodec::H264Avc,
                    90_000,
                    30,
                    0,
                    1_000_000_000,
                )
                .expect("track"),
            ],
            bytes.len() as u64,
            strong_sha256(bytes),
        )
        .expect("descriptor")
    }

    fn policy() -> EncryptedSpoolPolicy {
        EncryptedSpoolPolicy {
            max_total_bytes: 1024 * 1024,
            max_segment_bytes: 512 * 1024,
            max_files: 16,
        }
    }

    fn quota() -> SpoolQuotaPolicy {
        SpoolQuotaPolicy {
            max_retained_bytes: 1024 * 1024,
            max_reserved_bytes: 512 * 1024,
            max_segment_bytes: 512 * 1024,
        }
    }

    #[test]
    fn credential_key_decoder_rejects_wrong_length_and_zero_material() {
        assert!(matches!(
            decode_key_bytes(&[7_u8; 31]),
            Err(InstantError::SpoolKeyUnavailable)
        ));
        assert!(matches!(
            decode_key_bytes(&[0_u8; 32]),
            Err(InstantError::SpoolKeyUnavailable)
        ));
        assert_eq!(
            decode_key_bytes(&[9_u8; 32]).expect("valid key").as_ref(),
            &[9_u8; 32]
        );
    }

    #[test]
    fn encrypted_spool_round_trip_restart_tamper_and_cleanup() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keys = TestKeyStore::default();
        let session = InstantSessionId::from_csprng([7; 16]).expect("session");
        let body = b"secret media bytes that must never appear on disk".repeat(128);
        let descriptor = descriptor(session, &body);
        let port =
            FilesystemEncryptedSpool::new(directory.path(), policy(), keys.clone()).expect("port");
        let mut spool = InstantSpool::open(port, session, quota()).expect("spool");
        let receipt = spool
            .commit_segment(
                InstantOperationId::from_csprng([8; 16]).expect("operation"),
                &descriptor,
                Box::new(BytesPayload {
                    bytes: body.clone(),
                    offset: 0,
                }),
            )
            .expect("commit");
        assert!(receipt.durable);
        let disk = fs::read_dir(directory.path().join(hex(&session.canonical_bytes())))
            .expect("session directory")
            .map(|entry| fs::read(entry.expect("entry").path()).expect("spool bytes"))
            .next()
            .expect("spool file");
        assert!(!disk.windows(body.len()).any(|window| window == body));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let session_directory = directory.path().join(hex(&session.canonical_bytes()));
            assert_eq!(
                fs::metadata(&session_directory)
                    .expect("session metadata")
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
            let spool_path = fs::read_dir(&session_directory)
                .expect("session files")
                .map(|entry| entry.expect("session file").path())
                .find(|path| path.extension().and_then(|value| value.to_str()) == Some("spool"))
                .expect("spool path");
            assert_eq!(
                fs::metadata(spool_path)
                    .expect("spool metadata")
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }

        let mut upload = spool.open_upload(&descriptor).expect("open upload");
        let mut recovered_body = Vec::new();
        while let Some(chunk) = upload.next_chunk().expect("decrypt") {
            recovered_body.extend_from_slice(&chunk);
        }
        assert_eq!(recovered_body, body);

        drop(spool);
        let restarted = FilesystemEncryptedSpool::new(directory.path(), policy(), keys.clone())
            .expect("restart port");
        let recovered = InstantSpool::open(restarted, session, quota())
            .expect("restart open")
            .recover()
            .expect("recover");
        assert_eq!(recovered.retained_bytes(), body.len() as u64);
        drop(recovered);

        let session_directory = directory.path().join(hex(&session.canonical_bytes()));
        let path = fs::read_dir(&session_directory)
            .expect("read session")
            .map(|entry| entry.expect("entry").path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("spool"))
            .expect("spool path");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open tamper");
        file.seek(SeekFrom::Start(HEADER_BYTES + FRAME_HEADER_BYTES + 2))
            .expect("seek");
        file.write_all(&[0xff]).expect("tamper");
        file.sync_all().expect("sync tamper");
        let tampered =
            FilesystemEncryptedSpool::new(directory.path(), policy(), keys).expect("tamper port");
        assert!(matches!(
            InstantSpool::open(tampered, session, quota())
                .expect("tamper open")
                .recover(),
            Err(InstantError::SpoolCorrupt)
        ));
    }

    #[test]
    fn abandoned_reservation_is_removed_and_quota_is_fail_closed() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keys = TestKeyStore::default();
        let session = InstantSessionId::from_csprng([3; 16]).expect("session");
        let mut port =
            FilesystemEncryptedSpool::new(directory.path(), policy(), keys).expect("port");
        let key = port.acquire_runtime_key(session).expect("key");
        let claim = SpoolWriteClaim {
            session_id: session,
            operation_id: InstantOperationId::from_csprng([4; 16]).expect("operation"),
            segment_index: 0,
            segment_identity: strong_sha256(b"identity"),
            bytes: 32,
        };
        {
            let mut reservation = port.reserve(&key, claim).expect("reserve");
            reservation.write(b"partial").expect("partial write");
        }
        let session_directory = port.session_directory(session);
        assert!(
            fs::read_dir(session_directory)
                .expect("session entries")
                .next()
                .is_none()
        );

        let oversized = SpoolWriteClaim {
            bytes: policy().max_segment_bytes + 1,
            ..claim
        };
        assert!(matches!(
            port.reserve(&key, oversized),
            Err(InstantError::SpoolQuotaExceeded)
        ));
    }

    #[test]
    fn committed_ciphertext_without_credential_never_mints_a_replacement_key() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keys = TestKeyStore::default();
        let session = InstantSessionId::from_csprng([11; 16]).expect("session");
        let body = b"durable encrypted segment".repeat(32);
        let segment = descriptor(session, &body);
        let port =
            FilesystemEncryptedSpool::new(directory.path(), policy(), keys.clone()).expect("port");
        let mut spool = InstantSpool::open(port, session, quota()).expect("spool");
        spool
            .commit_segment(
                InstantOperationId::from_csprng([12; 16]).expect("operation"),
                &segment,
                Box::new(BytesPayload {
                    bytes: body,
                    offset: 0,
                }),
            )
            .expect("commit");
        drop(spool);

        keys.keys.lock().expect("key store").clear();
        let restarted =
            FilesystemEncryptedSpool::new(directory.path(), policy(), keys.clone()).expect("port");
        assert!(matches!(
            InstantSpool::open(restarted, session, quota()),
            Err(InstantError::SpoolKeyUnavailable)
        ));
        assert!(keys.keys.lock().expect("key store").is_empty());
    }
}
