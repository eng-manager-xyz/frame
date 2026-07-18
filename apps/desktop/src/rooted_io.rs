#![forbid(unsafe_code)]

//! Descriptor-rooted filesystem operations for the macOS recording spool.
//!
//! Every caller-supplied path component is opened relative to an already
//! trusted directory descriptor with `O_NOFOLLOW`. This keeps operations
//! attached to the directory that was originally bound, even if its visible
//! path is later renamed or replaced.
//!
//! Conditional cleanup and publish compare filesystem identities immediately
//! before mutating a path. macOS has no atomic "unlink this inode" operation,
//! so callers must place mutable staging names in a private directory (for
//! example, one returned by [`RootedDir::create_private_dir`]) and prevent
//! concurrent mutation by other threads between helper calls.

use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, RenameFlags, Stat, fchmod, fstat, fsync, mkdirat, open,
    openat, renameat_with, statat, unlinkat,
};
use rustix::io::{Errno, fcntl_dupfd_cloexec};
use thiserror::Error;

const MAX_ABSOLUTE_PATH_BYTES: usize = 4_096;
const MAX_RELATIVE_PATH_BYTES: usize = 4_096;
const MAX_COMPONENT_BYTES: usize = 255;
const MAX_ABSOLUTE_COMPONENTS: usize = 64;
const MAX_RELATIVE_COMPONENTS: usize = 32;

const FILE_MODE: Mode = Mode::from_raw_mode(0o600);
const DIRECTORY_MODE: Mode = Mode::from_raw_mode(0o700);

/// Errors returned by descriptor-rooted filesystem operations.
#[derive(Debug, Error)]
pub enum RootedIoError {
    /// The trusted root did not use the required absolute lexical form.
    #[error("invalid rooted directory path: {0}")]
    InvalidRoot(&'static str),

    /// A path beneath the root was empty, absolute, unbounded, or contained a
    /// non-normal component.
    #[error("invalid relative path: {0}")]
    InvalidRelativePath(&'static str),

    /// A filesystem call failed.
    #[error("{operation} failed: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: Errno,
    },

    /// A file operation encountered a non-regular filesystem object.
    #[error("path does not identify a regular file")]
    NotRegularFile,

    /// A directory operation encountered a non-directory filesystem object.
    #[error("path does not identify a directory")]
    NotDirectory,

    /// A filesystem object no longer has the identity supplied by the caller.
    #[error("filesystem identity changed before the operation")]
    IdentityMismatch,

    /// The filesystem returned a value that cannot be represented by this API.
    #[error("filesystem metadata is outside supported bounds")]
    InvalidMetadata,

    /// Exclusive creation found an existing filesystem object.
    #[error("filesystem entry already exists")]
    EntryExists,

    /// No-replace publication found an existing destination of any type.
    #[error("publish destination already exists")]
    DestinationExists,
}

/// Result type for descriptor-rooted filesystem operations.
pub type RootedIoResult<T> = Result<T, RootedIoError>;

/// Stable identity of a filesystem object on one mounted filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    /// Device number reported by `fstat`.
    #[must_use]
    pub const fn device(self) -> u64 {
        self.device
    }

    /// Inode number reported by `fstat`.
    #[must_use]
    pub const fn inode(self) -> u64 {
        self.inode
    }
}

/// Stable identity of a pinned directory on one mounted filesystem.
///
/// The identity is captured from the same descriptor retained by [`RootedDir`]
/// and therefore remains attached to the originally bound directory if its
/// visible path is renamed or replaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

impl DirectoryIdentity {
    /// Device number reported by `fstat` when the directory was pinned.
    #[must_use]
    pub const fn device(self) -> u64 {
        self.device
    }

    /// Inode number reported by `fstat` when the directory was pinned.
    #[must_use]
    pub const fn inode(self) -> u64 {
        self.inode
    }
}

/// Metadata captured from the same descriptor as an opened regular file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegularFileMetadata {
    identity: FileIdentity,
    size_bytes: u64,
}

impl RegularFileMetadata {
    /// Filesystem identity of the opened file.
    #[must_use]
    pub const fn identity(self) -> FileIdentity {
        self.identity
    }

    /// File length captured when the descriptor was opened or refreshed.
    #[must_use]
    pub const fn size_bytes(self) -> u64 {
        self.size_bytes
    }
}

/// A regular file opened without following any path symlink.
#[derive(Debug)]
pub struct RootedFile {
    file: File,
    metadata: RegularFileMetadata,
}

impl RootedFile {
    /// Borrow the underlying standard-library file.
    #[must_use]
    pub const fn file(&self) -> &File {
        &self.file
    }

    /// Mutably borrow the underlying standard-library file.
    pub const fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    /// Metadata captured from this descriptor.
    #[must_use]
    pub const fn metadata(&self) -> RegularFileMetadata {
        self.metadata
    }

    /// Refresh size and verify that the descriptor still identifies the same
    /// regular file.
    pub fn refresh_metadata(&mut self) -> RootedIoResult<RegularFileMetadata> {
        let refreshed = regular_file_metadata(
            &fstat(&self.file)
                .map_err(|source| io_error("refresh regular-file metadata", source))?,
        )?;
        if refreshed.identity != self.metadata.identity {
            return Err(RootedIoError::IdentityMismatch);
        }
        self.metadata = refreshed;
        Ok(refreshed)
    }

    /// Flush file data and metadata through `fsync`.
    pub fn sync(&self) -> RootedIoResult<()> {
        fsync(&self.file).map_err(|source| io_error("fsync regular file", source))
    }

    /// Consume the wrapper and return the standard-library file.
    #[must_use]
    pub fn into_file(self) -> File {
        self.file
    }
}

/// A directory pinned by an open descriptor.
#[derive(Debug)]
pub struct RootedDir {
    directory: OwnedFd,
    identity: DirectoryIdentity,
}

impl RootedDir {
    /// Bind an existing absolute directory without following a symlink in any
    /// component. The returned descriptor, not the visible path, is the root
    /// of all later operations.
    pub fn bind(path: impl AsRef<Path>) -> RootedIoResult<Self> {
        let components = validate_absolute_path(path.as_ref())?;
        let mut current = open("/", directory_open_flags(), Mode::empty())
            .map_err(|source| io_error("open filesystem root", source))?;

        for component in components {
            current = open_directory_at(&current, component)?;
        }

        let stat =
            fstat(&current).map_err(|source| io_error("inspect rooted directory", source))?;
        let identity = directory_identity(&stat)?;
        Ok(Self {
            directory: current,
            identity,
        })
    }

    /// Identity captured from this directory's pinned descriptor.
    #[must_use]
    pub const fn identity(&self) -> DirectoryIdentity {
        self.identity
    }

    /// Return whether an absolute visible path still names this pinned
    /// directory.
    ///
    /// Every path component is rebound with `O_NOFOLLOW`. A symlink or invalid
    /// path is rejected rather than treated as an identity match, including a
    /// symlink that resolves to the pinned directory.
    pub fn matches_visible_path(&self, path: impl AsRef<Path>) -> RootedIoResult<bool> {
        let visible = Self::bind(path)?;
        Ok(visible.identity == self.identity)
    }

    /// Open an existing regular file for reading without following a symlink
    /// in either its leaf or any intermediate component.
    pub fn open_regular_file(&self, relative: impl AsRef<Path>) -> RootedIoResult<RootedFile> {
        let relative = ValidatedRelativePath::new(relative.as_ref())?;
        let (parent_components, leaf) = relative.parent_and_leaf();
        let parent = self.walk_directories(parent_components)?;
        let descriptor = openat(
            &parent,
            leaf,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|source| io_error("open rooted regular file", source))?;
        rooted_file_from_descriptor(descriptor)
    }

    /// Open and retain an existing directory beneath this root without
    /// following a symlink in any component.
    pub fn open_dir(&self, relative: impl AsRef<Path>) -> RootedIoResult<Self> {
        let relative = ValidatedRelativePath::new(relative.as_ref())?;
        let directory = self.walk_directories(&relative.components)?;
        let stat = fstat(&directory)
            .map_err(|source| io_error("inspect opened rooted directory", source))?;
        let identity = directory_identity(&stat)?;
        Ok(Self {
            directory,
            identity,
        })
    }

    /// Set this pinned directory to exact mode `0700`, verify the result, and
    /// flush the metadata change.
    pub fn ensure_private_mode(&self) -> RootedIoResult<()> {
        fchmod(&self.directory, DIRECTORY_MODE)
            .map_err(|source| io_error("set private rooted-directory mode", source))?;
        let stat = fstat(&self.directory)
            .map_err(|source| io_error("verify private rooted-directory mode", source))?;
        ensure_directory(&stat)?;
        if Mode::from_raw_mode(stat.st_mode) != DIRECTORY_MODE {
            return Err(RootedIoError::InvalidMetadata);
        }
        self.sync()
    }

    /// Exclusively create a new regular file with exact mode `0600`.
    ///
    /// The parent directory must already exist and is also walked without
    /// following symlinks.
    pub fn create_new_file(&self, relative: impl AsRef<Path>) -> RootedIoResult<RootedFile> {
        let relative = ValidatedRelativePath::new(relative.as_ref())?;
        let (parent_components, leaf) = relative.parent_and_leaf();
        let parent = self.walk_directories(parent_components)?;
        let descriptor = match openat(
            &parent,
            leaf,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            FILE_MODE,
        ) {
            Ok(descriptor) => descriptor,
            Err(Errno::EXIST) => return Err(RootedIoError::EntryExists),
            Err(source) => return Err(io_error("create rooted regular file", source)),
        };

        let metadata = regular_file_metadata(
            &fstat(&descriptor)
                .map_err(|source| io_error("inspect created regular file", source))?,
        )?;

        if let Err(source) = fchmod(&descriptor, FILE_MODE) {
            cleanup_created_file(&parent, leaf, metadata.identity);
            return Err(io_error("set private regular-file mode", source));
        }
        if let Err(source) = fsync(&parent) {
            cleanup_created_file(&parent, leaf, metadata.identity);
            return Err(io_error("fsync new regular-file parent", source));
        }

        Ok(RootedFile {
            file: File::from(descriptor),
            metadata,
        })
    }

    /// Exclusively create and bind a new directory with exact mode `0700`.
    pub fn create_private_dir(&self, relative: impl AsRef<Path>) -> RootedIoResult<Self> {
        let relative = ValidatedRelativePath::new(relative.as_ref())?;
        let (parent_components, leaf) = relative.parent_and_leaf();
        let parent = self.walk_directories(parent_components)?;

        match mkdirat(&parent, leaf, DIRECTORY_MODE) {
            Ok(()) => {}
            Err(Errno::EXIST) => return Err(RootedIoError::EntryExists),
            Err(source) => return Err(io_error("create private rooted directory", source)),
        }

        let path_stat = match statat(&parent, leaf, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(source) => return Err(io_error("inspect created rooted directory", source)),
        };
        ensure_directory(&path_stat)?;
        let expected_identity = file_identity(&path_stat)?;

        let descriptor = match open_directory_at(&parent, leaf) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                cleanup_created_directory(&parent, leaf, expected_identity);
                return Err(error);
            }
        };
        let descriptor_stat = match fstat(&descriptor) {
            Ok(stat) => stat,
            Err(source) => {
                cleanup_created_directory(&parent, leaf, expected_identity);
                return Err(io_error(
                    "inspect private rooted directory descriptor",
                    source,
                ));
            }
        };
        if ensure_directory(&descriptor_stat).is_err() {
            cleanup_created_directory(&parent, leaf, expected_identity);
            return Err(RootedIoError::NotDirectory);
        }
        let descriptor_identity = match file_identity(&descriptor_stat) {
            Ok(identity) => identity,
            Err(error) => {
                cleanup_created_directory(&parent, leaf, expected_identity);
                return Err(error);
            }
        };
        if descriptor_identity != expected_identity {
            cleanup_created_directory(&parent, leaf, expected_identity);
            return Err(RootedIoError::IdentityMismatch);
        }

        if let Err(source) = fchmod(&descriptor, DIRECTORY_MODE) {
            cleanup_created_directory(&parent, leaf, expected_identity);
            return Err(io_error("set private rooted-directory mode", source));
        }
        if let Err(source) = fsync(&parent) {
            cleanup_created_directory(&parent, leaf, expected_identity);
            return Err(io_error("fsync private rooted-directory parent", source));
        }

        Ok(Self {
            directory: descriptor,
            identity: directory_identity_from_file(descriptor_identity),
        })
    }

    /// Flush this directory's metadata through `fsync`.
    pub fn sync(&self) -> RootedIoResult<()> {
        fsync(&self.directory).map_err(|source| io_error("fsync rooted directory", source))
    }

    /// Delete a regular file only if the leaf still has the expected device
    /// and inode identity.
    pub fn cleanup_file_if_identity(
        &self,
        relative: impl AsRef<Path>,
        expected: FileIdentity,
    ) -> RootedIoResult<()> {
        let relative = ValidatedRelativePath::new(relative.as_ref())?;
        let (parent_components, leaf) = relative.parent_and_leaf();
        let parent = self.walk_directories(parent_components)?;
        let stat = statat(&parent, leaf, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|source| io_error("inspect cleanup candidate", source))?;
        let metadata = regular_file_metadata(&stat)?;
        if metadata.identity != expected {
            return Err(RootedIoError::IdentityMismatch);
        }

        unlinkat(&parent, leaf, AtFlags::empty())
            .map_err(|source| io_error("unlink cleanup candidate", source))?;
        fsync(&parent).map_err(|source| io_error("fsync cleanup parent", source))
    }

    /// Atomically publish an expected regular-file identity to an absent
    /// destination. Existing files, directories, and symlinks are never
    /// replaced.
    ///
    /// The caller must `fsync` the file before publication. This method uses
    /// macOS `RENAME_EXCL`, verifies the destination identity after the
    /// rename, then `fsync`s both directory descriptors.
    pub fn publish_file_if_identity(
        &self,
        source: impl AsRef<Path>,
        expected: FileIdentity,
        destination: impl AsRef<Path>,
    ) -> RootedIoResult<RegularFileMetadata> {
        self.publish_file_to_root_if_identity(source, expected, self, destination)
    }

    /// Atomically publish an expected regular-file identity from this pinned
    /// root to an absent destination beneath another pinned root.
    ///
    /// Existing files, directories, and symlinks are never replaced. The
    /// caller must `fsync` the source file before publication. Both roots and
    /// all intermediate directories remain descriptor-rooted throughout the
    /// operation; after the rename the destination identity is verified and
    /// both parent directories are flushed.
    pub fn publish_file_to_root_if_identity(
        &self,
        source: impl AsRef<Path>,
        expected: FileIdentity,
        destination_root: &Self,
        destination: impl AsRef<Path>,
    ) -> RootedIoResult<RegularFileMetadata> {
        let source = ValidatedRelativePath::new(source.as_ref())?;
        let destination = ValidatedRelativePath::new(destination.as_ref())?;
        let (source_parent_components, source_leaf) = source.parent_and_leaf();
        let (destination_parent_components, destination_leaf) = destination.parent_and_leaf();
        let source_parent = self.walk_directories(source_parent_components)?;
        let destination_parent =
            destination_root.walk_directories(destination_parent_components)?;

        let source_stat = statat(&source_parent, source_leaf, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|source| io_error("inspect publish source", source))?;
        let source_metadata = regular_file_metadata(&source_stat)?;
        if source_metadata.identity != expected {
            return Err(RootedIoError::IdentityMismatch);
        }

        match statat(
            &destination_parent,
            destination_leaf,
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(_) => return Err(RootedIoError::DestinationExists),
            Err(Errno::NOENT) => {}
            Err(source) => return Err(io_error("inspect publish destination", source)),
        }

        match renameat_with(
            &source_parent,
            source_leaf,
            &destination_parent,
            destination_leaf,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {}
            Err(Errno::EXIST) => return Err(RootedIoError::DestinationExists),
            Err(source) => return Err(io_error("publish regular file", source)),
        }

        let published_stat = statat(
            &destination_parent,
            destination_leaf,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|source| io_error("verify published regular file", source))?;
        let published_metadata = regular_file_metadata(&published_stat)?;
        if published_metadata.identity != expected {
            return Err(RootedIoError::IdentityMismatch);
        }

        fsync(&source_parent).map_err(|source| io_error("fsync publish source parent", source))?;
        fsync(&destination_parent)
            .map_err(|source| io_error("fsync publish destination parent", source))?;
        Ok(published_metadata)
    }

    fn walk_directories(&self, components: &[&OsStr]) -> RootedIoResult<OwnedFd> {
        let mut current = fcntl_dupfd_cloexec(&self.directory, 0)
            .map_err(|source| io_error("duplicate rooted directory descriptor", source))?;
        for component in components {
            current = open_directory_at(&current, component)?;
        }
        Ok(current)
    }
}

#[derive(Debug)]
struct ValidatedRelativePath<'a> {
    components: Vec<&'a OsStr>,
}

impl<'a> ValidatedRelativePath<'a> {
    fn new(path: &'a Path) -> RootedIoResult<Self> {
        let bytes = path.as_os_str().as_bytes();
        validate_path_bytes(
            bytes,
            false,
            MAX_RELATIVE_PATH_BYTES,
            MAX_RELATIVE_COMPONENTS,
        )
        .map_err(RootedIoError::InvalidRelativePath)?;
        let components = bytes
            .split(|byte| *byte == b'/')
            .map(OsStr::from_bytes)
            .collect();
        Ok(Self { components })
    }

    fn parent_and_leaf(&self) -> (&[&'a OsStr], &'a OsStr) {
        let (leaf, parent) = self
            .components
            .split_last()
            .expect("validated relative paths always have a leaf");
        (parent, leaf)
    }
}

fn validate_absolute_path(path: &Path) -> RootedIoResult<Vec<&OsStr>> {
    let bytes = path.as_os_str().as_bytes();
    validate_path_bytes(
        bytes,
        true,
        MAX_ABSOLUTE_PATH_BYTES,
        MAX_ABSOLUTE_COMPONENTS,
    )
    .map_err(RootedIoError::InvalidRoot)?;
    if bytes == b"/" {
        return Ok(Vec::new());
    }
    Ok(bytes[1..]
        .split(|byte| *byte == b'/')
        .map(OsStr::from_bytes)
        .collect())
}

fn validate_path_bytes(
    bytes: &[u8],
    absolute: bool,
    maximum_bytes: usize,
    maximum_components: usize,
) -> Result<(), &'static str> {
    if bytes.is_empty() {
        return Err("path must not be empty");
    }
    if bytes.len() > maximum_bytes {
        return Err("path exceeds its byte bound");
    }
    if bytes.contains(&0) {
        return Err("path contains a NUL byte");
    }
    if absolute != bytes.starts_with(b"/") {
        return Err(if absolute {
            "path must be absolute"
        } else {
            "path must be relative"
        });
    }

    let component_bytes = if absolute { &bytes[1..] } else { bytes };
    if absolute && component_bytes.is_empty() {
        return Ok(());
    }
    if component_bytes.is_empty() || component_bytes.ends_with(b"/") {
        return Err("path must end in a normal component");
    }

    let mut component_count = 0_usize;
    for component in component_bytes.split(|byte| *byte == b'/') {
        component_count = component_count
            .checked_add(1)
            .ok_or("path component count overflowed")?;
        if component_count > maximum_components {
            return Err("path has too many components");
        }
        if component.is_empty() {
            return Err("path contains an empty component");
        }
        if component.len() > MAX_COMPONENT_BYTES {
            return Err("path component exceeds its byte bound");
        }
        if component == b"." || component == b".." {
            return Err("path contains a non-normal component");
        }
    }
    Ok(())
}

fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}

fn open_directory_at(parent: &OwnedFd, component: &OsStr) -> RootedIoResult<OwnedFd> {
    openat(parent, component, directory_open_flags(), Mode::empty())
        .map_err(|source| io_error("open rooted directory component", source))
}

fn rooted_file_from_descriptor(descriptor: OwnedFd) -> RootedIoResult<RootedFile> {
    let metadata = regular_file_metadata(
        &fstat(&descriptor).map_err(|source| io_error("inspect rooted regular file", source))?,
    )?;
    Ok(RootedFile {
        file: File::from(descriptor),
        metadata,
    })
}

fn regular_file_metadata(stat: &Stat) -> RootedIoResult<RegularFileMetadata> {
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(RootedIoError::NotRegularFile);
    }
    let size_bytes = u64::try_from(stat.st_size).map_err(|_| RootedIoError::InvalidMetadata)?;
    Ok(RegularFileMetadata {
        identity: file_identity(stat)?,
        size_bytes,
    })
}

fn ensure_directory(stat: &Stat) -> RootedIoResult<()> {
    if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
        Ok(())
    } else {
        Err(RootedIoError::NotDirectory)
    }
}

fn file_identity(stat: &Stat) -> RootedIoResult<FileIdentity> {
    Ok(FileIdentity {
        device: u64::from(stat.st_dev.cast_unsigned()),
        inode: stat.st_ino,
    })
}

fn directory_identity(stat: &Stat) -> RootedIoResult<DirectoryIdentity> {
    ensure_directory(stat)?;
    Ok(directory_identity_from_file(file_identity(stat)?))
}

const fn directory_identity_from_file(identity: FileIdentity) -> DirectoryIdentity {
    DirectoryIdentity {
        device: identity.device,
        inode: identity.inode,
    }
}

fn cleanup_created_file(parent: &OwnedFd, leaf: &OsStr, expected: FileIdentity) {
    let Ok(stat) = statat(parent, leaf, AtFlags::SYMLINK_NOFOLLOW) else {
        return;
    };
    let Ok(metadata) = regular_file_metadata(&stat) else {
        return;
    };
    if metadata.identity == expected {
        let _ = unlinkat(parent, leaf, AtFlags::empty());
    }
}

fn cleanup_created_directory(parent: &OwnedFd, leaf: &OsStr, expected: FileIdentity) {
    let Ok(stat) = statat(parent, leaf, AtFlags::SYMLINK_NOFOLLOW) else {
        return;
    };
    if ensure_directory(&stat).is_err() || file_identity(&stat).ok() != Some(expected) {
        return;
    }
    let _ = unlinkat(parent, leaf, AtFlags::REMOVEDIR);
}

fn io_error(operation: &'static str, source: Errno) -> RootedIoError {
    RootedIoError::Io { operation, source }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{RootedDir, RootedIoError};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "frame-rooted-io-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory should be created");
            let path = fs::canonicalize(path)
                .expect("test directory path should not contain system symlinks");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn bind_rejects_final_and_intermediate_symlink_components() {
        let temporary = TestDirectory::new("bind-symlinks");
        let real = temporary.path().join("real");
        let root = real.join("root");
        fs::create_dir(&real).expect("real parent should be created");
        fs::create_dir(&root).expect("real root should be created");

        symlink(&real, temporary.path().join("intermediate"))
            .expect("intermediate symlink should be created");
        symlink(&root, temporary.path().join("final")).expect("final symlink should be created");

        assert!(RootedDir::bind(temporary.path().join("intermediate/root")).is_err());
        assert!(RootedDir::bind(temporary.path().join("final")).is_err());
    }

    #[test]
    fn visible_path_matches_unchanged_pinned_directory() {
        let temporary = TestDirectory::new("visible-path-unchanged");
        let root_path = temporary.path().join("root");
        fs::create_dir(&root_path).expect("root should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");

        let identity = rooted.identity();
        let rebound = RootedDir::bind(&root_path).expect("unchanged root should rebind");
        assert_eq!(rebound.identity(), identity);
        assert_eq!(rebound.identity().device(), identity.device());
        assert_eq!(rebound.identity().inode(), identity.inode());
        assert!(
            rooted
                .matches_visible_path(&root_path)
                .expect("unchanged visible path should rebind")
        );
        assert_eq!(rooted.identity(), identity);
    }

    #[test]
    fn visible_path_detects_directory_rename_and_replacement() {
        let temporary = TestDirectory::new("visible-path-replacement");
        let original_path = temporary.path().join("root");
        let moved_path = temporary.path().join("moved");
        fs::create_dir(&original_path).expect("original root should be created");
        let rooted = RootedDir::bind(&original_path).expect("root should bind");

        fs::rename(&original_path, &moved_path).expect("bound root should be renamed");
        fs::create_dir(&original_path).expect("replacement root should be created");

        assert!(
            !rooted
                .matches_visible_path(&original_path)
                .expect("replacement directory should rebind")
        );
        assert!(
            rooted
                .matches_visible_path(&moved_path)
                .expect("renamed original directory should rebind")
        );
    }

    #[test]
    fn visible_path_rejects_symlink_replacement_even_to_pinned_directory() {
        let temporary = TestDirectory::new("visible-path-symlink-replacement");
        let original_path = temporary.path().join("root");
        let moved_path = temporary.path().join("moved");
        fs::create_dir(&original_path).expect("original root should be created");
        let rooted = RootedDir::bind(&original_path).expect("root should bind");

        fs::rename(&original_path, &moved_path).expect("bound root should be renamed");
        symlink(&moved_path, &original_path).expect("replacement symlink should be created");

        assert!(rooted.matches_visible_path(&original_path).is_err());
        assert!(
            rooted
                .matches_visible_path(&moved_path)
                .expect("renamed original directory should rebind")
        );
    }

    #[test]
    fn open_rejects_final_and_intermediate_symlinks() {
        let temporary = TestDirectory::new("open-symlinks");
        let root_path = temporary.path().join("root");
        let outside_path = temporary.path().join("outside");
        fs::create_dir(&root_path).expect("root should be created");
        fs::create_dir(&outside_path).expect("outside should be created");
        fs::write(outside_path.join("secret"), b"outside").expect("outside file should be written");
        symlink(outside_path.join("secret"), root_path.join("final"))
            .expect("final symlink should be created");
        symlink(&outside_path, root_path.join("intermediate"))
            .expect("intermediate symlink should be created");

        let rooted = RootedDir::bind(&root_path).expect("root should bind");
        assert!(rooted.open_regular_file("final").is_err());
        assert!(rooted.open_regular_file("intermediate/secret").is_err());
        assert!(rooted.open_dir("intermediate").is_err());
    }

    #[test]
    fn root_rename_and_replacement_do_not_retarget_descriptor() {
        let temporary = TestDirectory::new("root-replacement");
        let original_path = temporary.path().join("root");
        let moved_path = temporary.path().join("moved");
        fs::create_dir(&original_path).expect("original root should be created");
        fs::write(original_path.join("original"), b"original")
            .expect("original file should be written");
        let rooted = RootedDir::bind(&original_path).expect("root should bind");

        fs::rename(&original_path, &moved_path).expect("bound root should be renamed");
        fs::create_dir(&original_path).expect("replacement root should be created");
        fs::write(original_path.join("replacement"), b"replacement")
            .expect("replacement file should be written");

        let mut original = rooted
            .open_regular_file("original")
            .expect("bound descriptor should still see original file");
        let mut contents = String::new();
        original
            .file_mut()
            .read_to_string(&mut contents)
            .expect("original file should be readable");
        assert_eq!(contents, "original");

        let mut created = rooted
            .create_new_file("anchored")
            .expect("new file should be created under moved root");
        created
            .file_mut()
            .write_all(b"anchored")
            .expect("anchored file should be written");
        created.sync().expect("anchored file should sync");
        assert_eq!(
            fs::read(moved_path.join("anchored")).expect("moved-root file should exist"),
            b"anchored"
        );
        assert!(!original_path.join("anchored").exists());
    }

    #[test]
    fn create_rejects_existing_and_symlink_leaves() {
        let temporary = TestDirectory::new("create-existing");
        let root_path = temporary.path().join("root");
        let outside_path = temporary.path().join("outside");
        fs::create_dir(&root_path).expect("root should be created");
        fs::write(root_path.join("existing"), b"keep").expect("existing file should be written");
        fs::write(&outside_path, b"outside").expect("outside file should be written");
        symlink(&outside_path, root_path.join("link")).expect("symlink should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");

        assert!(matches!(
            rooted.create_new_file("existing"),
            Err(RootedIoError::EntryExists)
        ));
        assert!(matches!(
            rooted.create_new_file("link"),
            Err(RootedIoError::EntryExists)
        ));
        assert_eq!(
            fs::read(root_path.join("existing")).expect("existing file should remain"),
            b"keep"
        );
        assert_eq!(
            fs::read(&outside_path).expect("outside file should remain"),
            b"outside"
        );
    }

    #[test]
    fn private_creation_sets_exact_modes() {
        let temporary = TestDirectory::new("private-modes");
        let root_path = temporary.path().join("root");
        fs::create_dir(&root_path).expect("root should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");

        let file = rooted
            .create_new_file("private-file")
            .expect("private file should be created");
        let private = rooted
            .create_private_dir("private-directory")
            .expect("private directory should be created");
        fs::create_dir(root_path.join("existing-directory"))
            .expect("existing directory should be created");
        fs::set_permissions(
            root_path.join("existing-directory"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("existing directory mode should be widened");
        let existing = rooted
            .open_dir("existing-directory")
            .expect("existing directory should open without path symlinks");
        existing
            .ensure_private_mode()
            .expect("existing directory should become private");

        let file_mode = fs::metadata(root_path.join("private-file"))
            .expect("private file metadata should exist")
            .permissions()
            .mode()
            & 0o777;
        let directory_mode = fs::metadata(root_path.join("private-directory"))
            .expect("private directory metadata should exist")
            .permissions()
            .mode()
            & 0o777;
        let existing_directory_mode = fs::metadata(root_path.join("existing-directory"))
            .expect("existing directory metadata should exist")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(directory_mode, 0o700);
        assert_eq!(existing_directory_mode, 0o700);
        file.sync().expect("private file should sync");
        private.sync().expect("private directory should sync");
    }

    #[test]
    fn cleanup_refuses_to_delete_a_replacement() {
        let temporary = TestDirectory::new("cleanup-replacement");
        let root_path = temporary.path().join("root");
        fs::create_dir(&root_path).expect("root should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");
        let created = rooted
            .create_new_file("staging")
            .expect("staging file should be created");
        let expected = created.metadata().identity();

        fs::rename(root_path.join("staging"), root_path.join("old-staging"))
            .expect("original staging file should move");
        fs::write(root_path.join("staging"), b"replacement")
            .expect("replacement should be written");

        assert!(matches!(
            rooted.cleanup_file_if_identity("staging", expected),
            Err(RootedIoError::IdentityMismatch)
        ));
        assert_eq!(
            fs::read(root_path.join("staging")).expect("replacement should remain"),
            b"replacement"
        );
    }

    #[test]
    fn publish_refuses_existing_and_symlink_destinations() {
        let temporary = TestDirectory::new("publish-existing");
        let root_path = temporary.path().join("root");
        let outside_path = temporary.path().join("outside");
        fs::create_dir(&root_path).expect("root should be created");
        fs::write(root_path.join("existing"), b"keep").expect("existing file should be written");
        fs::write(&outside_path, b"outside").expect("outside file should be written");
        symlink(&outside_path, root_path.join("link")).expect("symlink should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");

        let first = rooted
            .create_new_file("first-staging")
            .expect("first staging file should be created");
        assert!(matches!(
            rooted.publish_file_if_identity(
                "first-staging",
                first.metadata().identity(),
                "existing"
            ),
            Err(RootedIoError::DestinationExists)
        ));
        assert_eq!(
            fs::read(root_path.join("existing")).expect("existing destination should remain"),
            b"keep"
        );
        assert!(root_path.join("first-staging").exists());

        let second = rooted
            .create_new_file("second-staging")
            .expect("second staging file should be created");
        assert!(matches!(
            rooted.publish_file_if_identity("second-staging", second.metadata().identity(), "link"),
            Err(RootedIoError::DestinationExists)
        ));
        assert!(
            fs::symlink_metadata(root_path.join("link"))
                .expect("destination symlink should remain")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(&outside_path).expect("outside file should remain"),
            b"outside"
        );
        assert!(root_path.join("second-staging").exists());
    }

    #[test]
    fn publish_moves_only_the_expected_identity() {
        let temporary = TestDirectory::new("publish-success");
        let root_path = temporary.path().join("root");
        fs::create_dir(&root_path).expect("root should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");
        let mut staging = rooted
            .create_new_file("staging")
            .expect("staging file should be created");
        staging
            .file_mut()
            .write_all(b"published")
            .expect("staging file should be written");
        staging.sync().expect("staging file should sync");
        let expected = staging.metadata().identity();

        let published_metadata = rooted
            .publish_file_if_identity("staging", expected, "final")
            .expect("staging file should publish");
        assert_eq!(published_metadata.identity(), expected);
        assert_eq!(published_metadata.size_bytes(), 9);
        assert!(!root_path.join("staging").exists());
        assert_eq!(
            fs::read(root_path.join("final")).expect("published file should exist"),
            b"published"
        );
        let published = rooted
            .open_regular_file("final")
            .expect("published file should reopen");
        assert_eq!(published.metadata().identity(), expected);
        assert_eq!(published.metadata().size_bytes(), 9);
    }

    #[test]
    fn publish_moves_expected_identity_between_sibling_pinned_roots() {
        let temporary = TestDirectory::new("publish-between-roots");
        let source_path = temporary.path().join("source");
        let destination_path = temporary.path().join("destination");
        fs::create_dir(&source_path).expect("source root should be created");
        fs::create_dir(&destination_path).expect("destination root should be created");
        let source_root = RootedDir::bind(&source_path).expect("source root should bind");
        let destination_root =
            RootedDir::bind(&destination_path).expect("destination root should bind");
        let mut staging = source_root
            .create_new_file("staging")
            .expect("staging file should be created");
        staging
            .file_mut()
            .write_all(b"published")
            .expect("staging file should be written");
        staging.sync().expect("staging file should sync");
        let expected = staging.metadata().identity();

        let published = source_root
            .publish_file_to_root_if_identity("staging", expected, &destination_root, "final")
            .expect("staging file should publish between pinned roots");

        assert_eq!(published.identity(), expected);
        assert_eq!(published.size_bytes(), 9);
        assert!(!source_path.join("staging").exists());
        assert_eq!(
            fs::read(destination_path.join("final")).expect("destination file should be readable"),
            b"published"
        );
        let reopened = destination_root
            .open_regular_file("final")
            .expect("destination should reopen through its pinned root");
        assert_eq!(reopened.metadata().identity(), expected);
    }

    #[test]
    fn publish_refuses_a_replaced_source() {
        let temporary = TestDirectory::new("publish-source-replacement");
        let root_path = temporary.path().join("root");
        fs::create_dir(&root_path).expect("root should be created");
        let rooted = RootedDir::bind(&root_path).expect("root should bind");
        let original = rooted
            .create_new_file("staging")
            .expect("staging file should be created");
        let expected = original.metadata().identity();

        fs::rename(root_path.join("staging"), root_path.join("old-staging"))
            .expect("original staging file should move");
        fs::write(root_path.join("staging"), b"replacement")
            .expect("replacement should be written");

        assert!(matches!(
            rooted.publish_file_if_identity("staging", expected, "final"),
            Err(RootedIoError::IdentityMismatch)
        ));
        assert_eq!(
            fs::read(root_path.join("staging")).expect("replacement should remain staged"),
            b"replacement"
        );
        assert!(!root_path.join("final").exists());
    }

    #[test]
    fn relative_paths_are_strict_and_bounded() {
        let temporary = TestDirectory::new("path-validation");
        let rooted = RootedDir::bind(temporary.path()).expect("root should bind");
        let too_many = std::iter::repeat_n("component", 33)
            .collect::<Vec<_>>()
            .join("/");
        let nul_path = Path::new(OsStr::from_bytes(b"nul\0leaf"));

        for invalid in ["", "/absolute", ".", "..", "a/./b", "a/../b", "a//b", "a/"] {
            assert!(
                matches!(
                    rooted.create_new_file(invalid),
                    Err(RootedIoError::InvalidRelativePath(_))
                ),
                "path should be rejected: {invalid:?}"
            );
        }
        assert!(matches!(
            rooted.create_new_file(too_many),
            Err(RootedIoError::InvalidRelativePath(_))
        ));
        assert!(matches!(
            rooted.create_new_file(nul_path),
            Err(RootedIoError::InvalidRelativePath(_))
        ));
    }
}
