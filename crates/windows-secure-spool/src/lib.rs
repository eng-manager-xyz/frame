//! Audited Windows boundary for the encrypted Instant spool.
//!
//! This crate is intentionally tiny: it owns the pointer-level calls needed
//! for Credential Manager, protected file ACLs, reparse-point detection, and
//! write-through publication. `frame-media` consumes only these safe wrappers.
//! No media, HTTP, provider, or application contract belongs here.

#![deny(unsafe_op_in_unsafe_fn)]

use std::fmt;

#[cfg(any(windows, test))]
use std::{ffi::OsStr, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsSecureSpoolError;

impl fmt::Display for WindowsSecureSpoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the Windows secure-spool operation failed")
    }
}

impl std::error::Error for WindowsSecureSpoolError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPublishError {
    AlreadyExists,
    Failed,
}

impl fmt::Display for WindowsPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => formatter.write_str("the Windows spool target already exists"),
            Self::Failed => formatter.write_str("the Windows spool publication failed"),
        }
    }
}

impl std::error::Error for WindowsPublishError {}

#[cfg(any(windows, test))]
fn destination_leaf(destination: &Path) -> Result<&OsStr, WindowsPublishError> {
    let parent = destination.parent().ok_or(WindowsPublishError::Failed)?;
    if parent.as_os_str().is_empty() {
        return Err(WindowsPublishError::Failed);
    }
    let leaf = destination
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(WindowsPublishError::Failed)?;
    if leaf.is_empty()
        || leaf.len() > 255
        || matches!(leaf, "." | "..")
        || !leaf
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(WindowsPublishError::Failed);
    }
    Ok(OsStr::new(leaf))
}

#[cfg(windows)]
mod windows {
    use std::{
        fs, mem,
        os::windows::{
            ffi::OsStrExt,
            fs::MetadataExt,
            io::{FromRawHandle, RawHandle},
        },
        path::Path,
        ptr, slice,
    };

    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_RENAME_INFORMATION, FILE_RENAME_INFORMATION_0, FileRenameInformation,
        NtSetInformationFile,
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NOT_FOUND, ERROR_SUCCESS, GENERIC_READ,
            GENERIC_WRITE, GetLastError, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree,
            STATUS_OBJECT_NAME_COLLISION,
        },
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SetSecurityInfo,
            },
            Credentials::{
                CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
                CredReadW, CredWriteW,
            },
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
            GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{
            CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_WRITE_THROUGH, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
            FileAttributeTagInfo, FlushFileBuffers, GetFileInformationByHandleEx, OPEN_EXISTING,
            READ_CONTROL, SYNCHRONIZE, WRITE_DAC,
        },
        System::IO::IO_STATUS_BLOCK,
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    use zeroize::{Zeroize, Zeroizing};

    use super::{WindowsPublishError, WindowsSecureSpoolError, destination_leaf};

    #[derive(Clone, Copy)]
    enum ExpectedObject {
        File,
        Directory,
        FileOrDirectory,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[allow(dead_code)]
    pub(super) enum PrivateAclStage {
        EncodePath,
        OpenPath,
        QueryAttributes,
        OpenProcessToken,
        SizeTokenUser,
        ReadTokenUser,
        EncodeSid,
        ParseSecurityDescriptor,
        ReadDacl,
        SetDacl,
        VerifyDacl,
        Invariant,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[allow(dead_code)]
    pub(super) struct PrivateAclDiagnostic {
        stage: PrivateAclStage,
        win32_status: Option<u32>,
    }

    impl PrivateAclDiagnostic {
        const fn invariant(stage: PrivateAclStage) -> Self {
            Self {
                stage,
                win32_status: None,
            }
        }

        fn last_error(stage: PrivateAclStage) -> Self {
            // SAFETY: callers invoke this immediately after the failing Win32
            // call and before any other FFI boundary can replace last-error.
            Self::status(stage, unsafe { GetLastError() })
        }

        const fn status(stage: PrivateAclStage, win32_status: u32) -> Self {
            Self {
                stage,
                win32_status: Some(win32_status),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[allow(dead_code)]
    pub(super) enum PublishStage {
        EncodeDestination,
        OpenSource,
        ValidateSource,
        OpenDestinationDirectory,
        ValidateDestinationDirectory,
        EncodeRename,
        FlushBeforeRename,
        Rename,
        FlushAfterRename,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum PublishDiagnostic {
        AlreadyExists,
        Failed {
            stage: PublishStage,
            platform_status: Option<u32>,
        },
    }

    impl PublishDiagnostic {
        const fn invariant(stage: PublishStage) -> Self {
            Self::Failed {
                stage,
                platform_status: None,
            }
        }

        fn last_error(stage: PublishStage) -> Self {
            // SAFETY: callers invoke this immediately after the failing Win32
            // call and before any other FFI boundary can replace last-error.
            Self::status(stage, unsafe { GetLastError() })
        }

        const fn status(stage: PublishStage, platform_status: u32) -> Self {
            Self::Failed {
                stage,
                platform_status: Some(platform_status),
            }
        }

        const fn from_acl(stage: PublishStage, diagnostic: PrivateAclDiagnostic) -> Self {
            Self::Failed {
                stage,
                platform_status: diagnostic.win32_status,
            }
        }
    }

    pub fn credential_load(
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<[u8; 32]>>, WindowsSecureSpoolError> {
        let target = credential_target(service, account)?;
        let mut credential: *mut CREDENTIALW = ptr::null_mut();
        // SAFETY: `target` is NUL-terminated for the call and `credential` is
        // a valid out pointer. A successful allocation is immediately guarded.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
            // SAFETY: this reads the error from the immediately preceding call.
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(WindowsSecureSpoolError)
            };
        }
        let credential = CredentialAllocation::new(credential)?;
        let raw = credential.get();
        if raw.CredentialBlobSize != 32 || raw.CredentialBlob.is_null() {
            return Err(WindowsSecureSpoolError);
        }
        // SAFETY: Credential Manager reports a 32-byte blob owned by the live
        // allocation. It is copied into a zeroizing value and wiped in place
        // before CredFree releases the allocation.
        let blob = unsafe { slice::from_raw_parts_mut(raw.CredentialBlob, 32) };
        let mut key = Zeroizing::new([0_u8; 32]);
        key.copy_from_slice(blob);
        blob.zeroize();
        if key.iter().all(|byte| *byte == 0) {
            return Err(WindowsSecureSpoolError);
        }
        Ok(Some(key))
    }

    pub fn credential_store(
        service: &str,
        account: &str,
        key: &[u8; 32],
    ) -> Result<(), WindowsSecureSpoolError> {
        if key.iter().all(|byte| *byte == 0) {
            return Err(WindowsSecureSpoolError);
        }
        let mut target = credential_target(service, account)?;
        let mut username = wide_string(account, 128)?;
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: 32,
            CredentialBlob: key.as_ptr().cast_mut(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            Comment: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            Attributes: ptr::null_mut(),
            ..CREDENTIALW::default()
        };
        // SAFETY: every non-null pointer references a live buffer for this
        // synchronous call; Windows copies the credential into its vault.
        (unsafe { CredWriteW(&credential, 0) } != 0)
            .then_some(())
            .ok_or(WindowsSecureSpoolError)
    }

    pub fn credential_delete(service: &str, account: &str) -> Result<(), WindowsSecureSpoolError> {
        let target = credential_target(service, account)?;
        // SAFETY: `target` is a live NUL-terminated UTF-16 string.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }
        // SAFETY: this reads the error from the immediately preceding call.
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            Ok(())
        } else {
            Err(WindowsSecureSpoolError)
        }
    }

    pub fn metadata_is_indirect(metadata: &fs::Metadata) -> bool {
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    pub fn create_private_file(path: &Path) -> Result<fs::File, WindowsSecureSpoolError> {
        let user_sid = current_user_sid().map_err(|_| WindowsSecureSpoolError)?;
        let descriptor =
            security_descriptor(&format!("D:P(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)"))
                .map_err(|_| WindowsSecureSpoolError)?;
        let path = path_wide(path)?;
        let security_attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>())
                .map_err(|_| WindowsSecureSpoolError)?,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        // SAFETY: the path and self-relative security descriptor stay live for
        // the synchronous call. CREATE_NEW makes final-component creation
        // atomic, and Windows copies the descriptor into the new file object.
        let raw = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &security_attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH,
                ptr::null_mut(),
            )
        };
        let handle = Handle::new(raw)?;
        validate_handle(&handle, ExpectedObject::File).map_err(|_| WindowsSecureSpoolError)?;
        Ok(handle.into_file())
    }

    pub fn enforce_private_permissions(path: &Path) -> Result<(), WindowsSecureSpoolError> {
        enforce_private_permissions_inner(path).map_err(|_| WindowsSecureSpoolError)
    }

    fn enforce_private_permissions_inner(path: &Path) -> Result<(), PrivateAclDiagnostic> {
        let handle = open_path(
            path,
            WRITE_DAC | READ_CONTROL | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
        )?;
        validate_handle(&handle, ExpectedObject::FileOrDirectory)?;
        apply_private_dacl(&handle)
    }

    #[cfg(test)]
    pub(super) fn diagnose_private_permissions(path: &Path) -> Result<(), PrivateAclDiagnostic> {
        enforce_private_permissions_inner(path)
    }

    fn apply_private_dacl(handle: &Handle) -> Result<(), PrivateAclDiagnostic> {
        let user_sid = current_user_sid()?;
        let descriptor =
            security_descriptor(&format!("D:P(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)"))?;
        let mut dacl_present = 0;
        let mut dacl_defaulted = 0;
        let mut dacl = ptr::null_mut();
        // SAFETY: the guarded descriptor is valid and owns the returned DACL.
        if unsafe {
            GetSecurityDescriptorDacl(
                descriptor.0,
                &mut dacl_present,
                &mut dacl,
                &mut dacl_defaulted,
            )
        } == 0
            || dacl_present == 0
            || dacl.is_null()
        {
            return Err(PrivateAclDiagnostic::invariant(PrivateAclStage::ReadDacl));
        }
        // SAFETY: the DACL is owned by the live descriptor and the handle pins
        // the already-validated object. The API retains neither value.
        let status = unsafe {
            SetSecurityInfo(
                handle.0,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(PrivateAclDiagnostic::status(
                PrivateAclStage::SetDacl,
                status,
            ));
        }
        verify_private_dacl(handle)
    }

    fn verify_private_dacl(handle: &Handle) -> Result<(), PrivateAclDiagnostic> {
        let mut dacl = ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: every out pointer is valid, the handle stays live, and the
        // returned descriptor is immediately guarded with LocalFree ownership.
        let status = unsafe {
            GetSecurityInfo(
                handle.0,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(PrivateAclDiagnostic::status(
                PrivateAclStage::VerifyDacl,
                status,
            ));
        }
        if descriptor.is_null() || dacl.is_null() {
            return Err(PrivateAclDiagnostic::invariant(PrivateAclStage::VerifyDacl));
        }
        let descriptor = LocalAllocation(descriptor);
        let mut control = 0_u16;
        let mut revision = 0_u32;
        // SAFETY: the descriptor allocation is live and both scalar out
        // pointers are valid for the synchronous control query.
        if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
            return Err(PrivateAclDiagnostic::last_error(
                PrivateAclStage::VerifyDacl,
            ));
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Err(PrivateAclDiagnostic::invariant(PrivateAclStage::VerifyDacl));
        }
        Ok(())
    }

    pub fn publish_file(source: &Path, destination: &Path) -> Result<(), WindowsPublishError> {
        match publish_file_inner(source, destination) {
            Ok(()) => Ok(()),
            Err(PublishDiagnostic::AlreadyExists) => Err(WindowsPublishError::AlreadyExists),
            Err(PublishDiagnostic::Failed { .. }) => Err(WindowsPublishError::Failed),
        }
    }

    #[cfg(test)]
    pub(super) fn diagnose_publish_file(
        source: &Path,
        destination: &Path,
    ) -> Result<(), PublishDiagnostic> {
        publish_file_inner(source, destination)
    }

    fn publish_file_inner(source: &Path, destination: &Path) -> Result<(), PublishDiagnostic> {
        let leaf = destination_leaf(destination)
            .map_err(|_| PublishDiagnostic::invariant(PublishStage::EncodeDestination))?;
        let parent = destination.parent().ok_or(PublishDiagnostic::invariant(
            PublishStage::EncodeDestination,
        ))?;
        let source = open_path(
            source,
            GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH,
        )
        .map_err(|error| PublishDiagnostic::from_acl(PublishStage::OpenSource, error))?;
        validate_handle(&source, ExpectedObject::File)
            .map_err(|error| PublishDiagnostic::from_acl(PublishStage::ValidateSource, error))?;
        let destination_directory = open_path(
            parent,
            FILE_TRAVERSE | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
        )
        .map_err(|error| {
            PublishDiagnostic::from_acl(PublishStage::OpenDestinationDirectory, error)
        })?;
        validate_handle(&destination_directory, ExpectedObject::Directory).map_err(|error| {
            PublishDiagnostic::from_acl(PublishStage::ValidateDestinationDirectory, error)
        })?;
        let leaf = leaf.encode_wide().collect::<Vec<_>>();
        let name_bytes = leaf
            .len()
            .checked_mul(mem::size_of::<u16>())
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(PublishDiagnostic::invariant(PublishStage::EncodeRename))?;
        let information_bytes = mem::size_of::<FILE_RENAME_INFORMATION>()
            .checked_add(
                usize::try_from(name_bytes)
                    .map_err(|_| PublishDiagnostic::invariant(PublishStage::EncodeRename))?,
            )
            .ok_or(PublishDiagnostic::invariant(PublishStage::EncodeRename))?;
        let information_size = u32::try_from(information_bytes)
            .map_err(|_| PublishDiagnostic::invariant(PublishStage::EncodeRename))?;
        let words = information_bytes
            .checked_add(mem::size_of::<usize>() - 1)
            .map(|bytes| bytes / mem::size_of::<usize>())
            .ok_or(PublishDiagnostic::invariant(PublishStage::EncodeRename))?;
        let mut information = vec![0_usize; words];
        let rename = information.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        // SAFETY: `information` is pointer-aligned and sized through the final
        // UTF-16 code unit. The relative leaf has no separator or stream colon;
        // the destination directory and source file remain pinned by handles.
        unsafe {
            ptr::addr_of_mut!((*rename).Anonymous).write(FILE_RENAME_INFORMATION_0 {
                ReplaceIfExists: false,
            });
            ptr::addr_of_mut!((*rename).RootDirectory).write(destination_directory.0);
            ptr::addr_of_mut!((*rename).FileNameLength).write(name_bytes);
            ptr::copy_nonoverlapping(
                leaf.as_ptr(),
                ptr::addr_of_mut!((*rename).FileName).cast::<u16>(),
                leaf.len(),
            );
        }
        // The writer synced before entering this boundary. Flush the exact
        // source handle once more before changing its directory entry.
        if unsafe { FlushFileBuffers(source.0) } == 0 {
            return Err(PublishDiagnostic::last_error(
                PublishStage::FlushBeforeRename,
            ));
        }
        let mut io_status = IO_STATUS_BLOCK::default();
        // SAFETY: the variable-sized FILE_RENAME_INFORMATION buffer is fully
        // initialized, both handles remain live, and this synchronous source
        // handle makes the returned NTSTATUS authoritative for the operation.
        let status = unsafe {
            NtSetInformationFile(
                source.0,
                &mut io_status,
                rename.cast(),
                information_size,
                FileRenameInformation,
            )
        };
        if status == STATUS_OBJECT_NAME_COLLISION {
            return Err(PublishDiagnostic::AlreadyExists);
        }
        if status < 0 {
            return Err(PublishDiagnostic::status(
                PublishStage::Rename,
                u32::from_ne_bytes(status.to_ne_bytes()),
            ));
        }
        // Flush through the same handle after its no-replace rename. Windows
        // exposes no unprivileged, documented directory-fsync primitive, so
        // power-loss behavior remains a protected platform evidence gate.
        if unsafe { FlushFileBuffers(source.0) } == 0 {
            return Err(PublishDiagnostic::last_error(
                PublishStage::FlushAfterRename,
            ));
        }
        Ok(())
    }

    fn open_path(
        path: &Path,
        access: u32,
        share: u32,
        flags: u32,
    ) -> Result<Handle, PrivateAclDiagnostic> {
        let path = path_wide(path)
            .map_err(|_| PrivateAclDiagnostic::invariant(PrivateAclStage::EncodePath))?;
        // SAFETY: the path is live and NUL-terminated. OPEN_EXISTING plus
        // OPEN_REPARSE_POINT returns a handle to the named final object rather
        // than following a final-component reparse point.
        let raw = unsafe {
            CreateFileW(
                path.as_ptr(),
                access,
                share,
                ptr::null(),
                OPEN_EXISTING,
                flags,
                ptr::null_mut(),
            )
        };
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(PrivateAclDiagnostic::last_error(PrivateAclStage::OpenPath))
        } else {
            Ok(Handle(raw))
        }
    }

    fn validate_handle(
        handle: &Handle,
        expected: ExpectedObject,
    ) -> Result<(), PrivateAclDiagnostic> {
        let mut information = FILE_ATTRIBUTE_TAG_INFO::default();
        let information_size = u32::try_from(mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>())
            .map_err(|_| PrivateAclDiagnostic::invariant(PrivateAclStage::Invariant))?;
        // SAFETY: the output points to a correctly sized live structure and the
        // handle remains owned for the call.
        if unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileAttributeTagInfo,
                ptr::addr_of_mut!(information).cast(),
                information_size,
            )
        } == 0
        {
            return Err(PrivateAclDiagnostic::last_error(
                PrivateAclStage::QueryAttributes,
            ));
        }
        if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(PrivateAclDiagnostic::invariant(
                PrivateAclStage::QueryAttributes,
            ));
        }
        let is_directory = information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        match expected {
            ExpectedObject::File if is_directory => {
                Err(PrivateAclDiagnostic::invariant(PrivateAclStage::Invariant))
            }
            ExpectedObject::Directory if !is_directory => {
                Err(PrivateAclDiagnostic::invariant(PrivateAclStage::Invariant))
            }
            _ => Ok(()),
        }
    }

    fn credential_target(
        service: &str,
        account: &str,
    ) -> Result<Vec<u16>, WindowsSecureSpoolError> {
        if !valid_component(service) || !valid_component(account) {
            return Err(WindowsSecureSpoolError);
        }
        wide_string(&format!("FrameInstantSpool:{service}:{account}"), 512)
    }

    fn valid_component(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    }

    fn wide_string(value: &str, maximum: usize) -> Result<Vec<u16>, WindowsSecureSpoolError> {
        if value.is_empty() || value.len() > maximum || value.contains('\0') {
            return Err(WindowsSecureSpoolError);
        }
        let mut wide = value.encode_utf16().collect::<Vec<_>>();
        wide.push(0);
        Ok(wide)
    }

    fn path_wide(path: &Path) -> Result<Vec<u16>, WindowsSecureSpoolError> {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if wide.is_empty() || wide.len() >= 32_767 || wide.contains(&0) {
            return Err(WindowsSecureSpoolError);
        }
        wide.push(0);
        Ok(wide)
    }

    fn current_user_sid() -> Result<String, PrivateAclDiagnostic> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: token is a valid out pointer and the process pseudo-handle
        // remains valid for this call.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(PrivateAclDiagnostic::last_error(
                PrivateAclStage::OpenProcessToken,
            ));
        }
        if token.is_null() || token == INVALID_HANDLE_VALUE {
            return Err(PrivateAclDiagnostic::invariant(
                PrivateAclStage::OpenProcessToken,
            ));
        }
        let token = Handle(token);
        let mut required = 0_u32;
        // SAFETY: sizing call writes only `required`.
        let sizing_result =
            unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required) };
        // SAFETY: this reads the error from the immediately preceding sizing
        // call before any other Win32 boundary can replace it.
        let sizing_status = unsafe { GetLastError() };
        if sizing_result != 0 {
            return Err(PrivateAclDiagnostic::invariant(
                PrivateAclStage::SizeTokenUser,
            ));
        }
        if sizing_status != ERROR_INSUFFICIENT_BUFFER {
            return Err(PrivateAclDiagnostic::status(
                PrivateAclStage::SizeTokenUser,
                sizing_status,
            ));
        }
        if required < mem::size_of::<TOKEN_USER>() as u32 || required > 64 * 1024 {
            return Err(PrivateAclDiagnostic::invariant(
                PrivateAclStage::SizeTokenUser,
            ));
        }
        let words = usize::try_from(required)
            .ok()
            .and_then(|bytes| bytes.checked_add(mem::size_of::<usize>() - 1))
            .map(|bytes| bytes / mem::size_of::<usize>())
            .ok_or(PrivateAclDiagnostic::invariant(PrivateAclStage::Invariant))?;
        let mut buffer = vec![0_usize; words];
        let buffer_bytes = u32::try_from(buffer.len() * mem::size_of::<usize>())
            .map_err(|_| PrivateAclDiagnostic::invariant(PrivateAclStage::Invariant))?;
        // SAFETY: the aligned buffer is writable for `buffer_bytes`; success
        // initializes TOKEN_USER and its SID within the allocation.
        let read_result = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                buffer_bytes,
                &mut required,
            )
        };
        if read_result == 0 {
            return Err(PrivateAclDiagnostic::last_error(
                PrivateAclStage::ReadTokenUser,
            ));
        }
        if required < mem::size_of::<TOKEN_USER>() as u32 || required > buffer_bytes {
            return Err(PrivateAclDiagnostic::invariant(
                PrivateAclStage::ReadTokenUser,
            ));
        }
        // SAFETY: the successful call initialized TOKEN_USER at the aligned
        // start of the still-live buffer.
        let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut encoded = ptr::null_mut();
        // SAFETY: the SID stays live in `buffer`; the returned allocation is
        // immediately guarded and later released with LocalFree.
        if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut encoded) } == 0 {
            return Err(PrivateAclDiagnostic::last_error(PrivateAclStage::EncodeSid));
        }
        if encoded.is_null() {
            return Err(PrivateAclDiagnostic::invariant(PrivateAclStage::EncodeSid));
        }
        let encoded = LocalAllocation(encoded.cast());
        let length = (0..512)
            // SAFETY: the API returns a NUL-terminated SID string whose
            // documented representation is well below this hard bound.
            .find(|offset| unsafe { *encoded.0.cast::<u16>().add(*offset) } == 0)
            .ok_or(PrivateAclDiagnostic::invariant(PrivateAclStage::EncodeSid))?;
        // SAFETY: `length` is bounded by the first NUL in the live allocation.
        let sid = String::from_utf16(unsafe { slice::from_raw_parts(encoded.0.cast(), length) })
            .map_err(|_| PrivateAclDiagnostic::invariant(PrivateAclStage::EncodeSid))?;
        if valid_sid_string(&sid) {
            Ok(sid)
        } else {
            Err(PrivateAclDiagnostic::invariant(PrivateAclStage::EncodeSid))
        }
    }

    fn valid_sid_string(value: &str) -> bool {
        value.len() <= 256
            && value.strip_prefix("S-1-").is_some_and(|components| {
                !components.is_empty()
                    && components.split('-').all(|component| {
                        !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
    }

    fn security_descriptor(value: &str) -> Result<LocalAllocation, PrivateAclDiagnostic> {
        let encoded = wide_string(value, 1_024).map_err(|_| {
            PrivateAclDiagnostic::invariant(PrivateAclStage::ParseSecurityDescriptor)
        })?;
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: input is NUL-terminated and `descriptor` is a valid out
        // pointer. Windows allocates the result with LocalAlloc.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                encoded.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(PrivateAclDiagnostic::last_error(
                PrivateAclStage::ParseSecurityDescriptor,
            ));
        }
        if descriptor.is_null() {
            return Err(PrivateAclDiagnostic::invariant(
                PrivateAclStage::ParseSecurityDescriptor,
            ));
        }
        Ok(LocalAllocation(descriptor))
    }

    struct CredentialAllocation(*mut CREDENTIALW);

    impl CredentialAllocation {
        fn new(value: *mut CREDENTIALW) -> Result<Self, WindowsSecureSpoolError> {
            (!value.is_null())
                .then_some(Self(value))
                .ok_or(WindowsSecureSpoolError)
        }

        fn get(&self) -> &CREDENTIALW {
            // SAFETY: construction rejects null; the guard owns the allocation.
            unsafe { &*self.0 }
        }
    }

    impl Drop for CredentialAllocation {
        fn drop(&mut self) {
            // SAFETY: CredReadW returned this allocation and it is freed once.
            unsafe { CredFree(self.0.cast()) };
        }
    }

    struct Handle(HANDLE);

    impl Handle {
        fn new(value: HANDLE) -> Result<Self, WindowsSecureSpoolError> {
            (!value.is_null() && value != INVALID_HANDLE_VALUE)
                .then_some(Self(value))
                .ok_or(WindowsSecureSpoolError)
        }

        fn into_file(mut self) -> fs::File {
            let handle = self.0;
            self.0 = ptr::null_mut();
            // SAFETY: this transfers the one owned CreateFileW handle into File.
            unsafe { fs::File::from_raw_handle(handle as RawHandle) }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                // SAFETY: a Win32 open call returned this owned handle.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    struct LocalAllocation(HLOCAL);

    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: these APIs document LocalAlloc ownership.
                unsafe { LocalFree(self.0) };
            }
        }
    }
}

#[cfg(windows)]
pub use windows::{
    create_private_file, credential_delete, credential_load, credential_store,
    enforce_private_permissions, metadata_is_indirect, publish_file,
};

#[cfg(all(test, windows))]
use windows::{diagnose_private_permissions, diagnose_publish_file};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{WindowsPublishError, destination_leaf};

    #[test]
    fn publication_leaf_is_a_bounded_ascii_component() {
        assert_eq!(
            destination_leaf(Path::new("root/000001-deadbeef.spool")).expect("valid leaf"),
            "000001-deadbeef.spool"
        );
        for invalid in [
            "target",
            "root/..",
            "root/name:stream",
            "root/name child",
            "root/é.spool",
        ] {
            assert_eq!(
                destination_leaf(Path::new(invalid)),
                Err(WindowsPublishError::Failed),
                "{invalid}"
            );
        }
        // Backslash is an invalid Unix leaf byte but a Windows path separator.
        // `destination_leaf` validates the component selected by `Path`, so the
        // platform parser—not a string heuristic—owns this distinction.
        #[cfg(not(windows))]
        assert_eq!(
            destination_leaf(Path::new("root/name\\child")),
            Err(WindowsPublishError::Failed)
        );
        #[cfg(windows)]
        assert_eq!(
            destination_leaf(Path::new("root/name\\child")).expect("nested Windows path"),
            "child"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        WindowsPublishError, create_private_file, diagnose_private_permissions,
        diagnose_publish_file, publish_file,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("frame-secure-spool-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).expect("test directory");
            diagnose_private_permissions(&path).expect("private test directory");
            Self(path)
        }

        fn join(&self, leaf: &str) -> PathBuf {
            self.0.join(leaf)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn private_creation_and_handle_relative_no_replace_publication() {
        let directory = TestDirectory::new();
        let source = directory.join("segment.tmp");
        let destination = directory.join("segment.spool");
        let mut file = create_private_file(&source).expect("private source");
        file.write_all(b"authenticated ciphertext")
            .expect("write source");
        file.sync_all().expect("sync source");

        diagnose_publish_file(&source, &destination).expect("publish");
        assert!(!source.exists());
        assert_eq!(
            fs::read(&destination).expect("published bytes"),
            b"authenticated ciphertext"
        );
        drop(file);

        let second_source = directory.join("second.tmp");
        let existing_target = directory.join("existing.spool");
        drop(create_private_file(&second_source).expect("second source"));
        drop(create_private_file(&existing_target).expect("existing target"));
        assert_eq!(
            publish_file(&second_source, &existing_target),
            Err(WindowsPublishError::AlreadyExists)
        );
        assert!(Path::new(&second_source).exists());
    }

    #[test]
    fn handle_relative_publication_pins_a_cross_directory_destination() {
        let source_directory = TestDirectory::new();
        let destination_directory = TestDirectory::new();
        let source = source_directory.join("segment.tmp");
        let destination = destination_directory.join("segment.spool");
        drop(create_private_file(&source).expect("private source"));

        diagnose_publish_file(&source, &destination).expect("cross-directory publish");
        assert!(!source.exists());
        assert!(destination.exists());
    }
}
