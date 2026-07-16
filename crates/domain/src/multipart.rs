use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ByteSize, ChecksumSha256, ContentType, DurationMillis, MultipartUploadId, ScopedObjectKey,
    SecretDigest, TenantId, TimestampMillis,
};

pub const MULTIPART_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MultipartContractError {
    #[error("multipart protocol version is unsupported")]
    UnsupportedVersion,
    #[error("multipart limit is invalid")]
    InvalidLimit,
    #[error("multipart upload specification is invalid")]
    InvalidUploadSpec,
    #[error("multipart part is invalid")]
    InvalidPart,
    #[error("multipart authorization grant is invalid")]
    InvalidGrant,
    #[error("trusted media probe is invalid")]
    InvalidMediaProbe,
    #[error("download origin is invalid")]
    InvalidCorsOrigin,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MultipartGrantId(Uuid);

impl MultipartGrantId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn parse(value: &str) -> Result<Self, MultipartContractError> {
        let value = Uuid::parse_str(value).map_err(|_| MultipartContractError::InvalidGrant)?;
        if value.is_nil() {
            return Err(MultipartContractError::InvalidGrant);
        }
        Ok(Self(value))
    }
}

impl Default for MultipartGrantId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MultipartGrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("MultipartGrantId")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for MultipartGrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MultipartGrantSecret(String);

impl MultipartGrantSecret {
    pub fn parse(value: impl Into<String>) -> Result<Self, MultipartContractError> {
        let value = value.into();
        if !(32..=256).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
        {
            return Err(MultipartContractError::InvalidGrant);
        }
        Ok(Self(value))
    }

    /// Exposes the bearer only at the keyed hashing boundary.
    #[must_use]
    pub fn expose_for_hashing(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for MultipartGrantSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MultipartGrantSecret([redacted])")
    }
}

impl fmt::Display for MultipartGrantSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct MultipartGrantKeyVersion(u16);

impl MultipartGrantKeyVersion {
    pub fn new(value: u16) -> Result<Self, MultipartContractError> {
        if value == 0 {
            return Err(MultipartContractError::InvalidGrant);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for MultipartGrantKeyVersion {
    type Error = MultipartContractError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<MultipartGrantKeyVersion> for u16 {
    fn from(value: MultipartGrantKeyVersion) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultipartOperationV1 {
    Create,
    ListParts,
    PutPart,
    Complete,
    Finalize,
    Abort,
    Head,
    Get,
    Range,
}

impl MultipartOperationV1 {
    #[must_use]
    pub const fn requires_upload(self) -> bool {
        matches!(
            self,
            Self::ListParts | Self::PutPart | Self::Complete | Self::Finalize | Self::Abort
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct MultipartPartNumberV1(u16);

impl MultipartPartNumberV1 {
    pub fn new(value: u16) -> Result<Self, MultipartContractError> {
        if !(1..=10_000).contains(&value) {
            return Err(MultipartContractError::InvalidPart);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for MultipartPartNumberV1 {
    type Error = MultipartContractError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<MultipartPartNumberV1> for u16 {
    fn from(value: MultipartPartNumberV1) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultipartLimitsV1 {
    min_part_size: ByteSize,
    max_part_size: ByteSize,
    max_part_count: u16,
    max_total_size: ByteSize,
    max_worker_request_size: ByteSize,
    max_grant_ttl: DurationMillis,
}

impl MultipartLimitsV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        min_part_size: ByteSize,
        max_part_size: ByteSize,
        max_part_count: u16,
        max_total_size: ByteSize,
        max_worker_request_size: ByteSize,
        max_grant_ttl: DurationMillis,
    ) -> Result<Self, MultipartContractError> {
        if min_part_size.get() == 0
            || min_part_size > max_part_size
            || max_part_size > max_worker_request_size
            || !(1..=10_000).contains(&max_part_count)
            || max_total_size.get() == 0
            || max_total_size < min_part_size
            || max_grant_ttl.get() == 0
        {
            return Err(MultipartContractError::InvalidLimit);
        }
        Ok(Self {
            min_part_size,
            max_part_size,
            max_part_count,
            max_total_size,
            max_worker_request_size,
            max_grant_ttl,
        })
    }

    #[must_use]
    pub const fn min_part_size(self) -> ByteSize {
        self.min_part_size
    }

    #[must_use]
    pub const fn max_part_size(self) -> ByteSize {
        self.max_part_size
    }

    #[must_use]
    pub const fn max_part_count(self) -> u16 {
        self.max_part_count
    }

    #[must_use]
    pub const fn max_total_size(self) -> ByteSize {
        self.max_total_size
    }

    #[must_use]
    pub const fn max_worker_request_size(self) -> ByteSize {
        self.max_worker_request_size
    }

    #[must_use]
    pub const fn max_grant_ttl(self) -> DurationMillis {
        self.max_grant_ttl
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartUploadSpecV1 {
    protocol_version: u16,
    key: ScopedObjectKey,
    total_size: ByteSize,
    part_size: ByteSize,
    part_count: u16,
    checksum_sha256: ChecksumSha256,
    content_type: ContentType,
}

impl MultipartUploadSpecV1 {
    pub fn new(
        key: ScopedObjectKey,
        total_size: ByteSize,
        part_size: ByteSize,
        checksum_sha256: ChecksumSha256,
        content_type: ContentType,
        limits: MultipartLimitsV1,
    ) -> Result<Self, MultipartContractError> {
        if total_size.get() == 0
            || total_size > limits.max_total_size
            || part_size < limits.min_part_size
            || part_size > limits.max_part_size
            || part_size > limits.max_worker_request_size
        {
            return Err(MultipartContractError::InvalidUploadSpec);
        }
        let part_count = total_size
            .get()
            .checked_add(part_size.get() - 1)
            .ok_or(MultipartContractError::InvalidUploadSpec)?
            / part_size.get();
        let part_count =
            u16::try_from(part_count).map_err(|_| MultipartContractError::InvalidUploadSpec)?;
        if part_count == 0 || part_count > limits.max_part_count {
            return Err(MultipartContractError::InvalidUploadSpec);
        }
        Ok(Self {
            protocol_version: MULTIPART_PROTOCOL_VERSION,
            key,
            total_size,
            part_size,
            part_count,
            checksum_sha256,
            content_type,
        })
    }

    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn total_size(&self) -> ByteSize {
        self.total_size
    }

    #[must_use]
    pub const fn part_size(&self) -> ByteSize {
        self.part_size
    }

    #[must_use]
    pub const fn part_count(&self) -> u16 {
        self.part_count
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    pub fn expected_part_size(
        &self,
        part_number: MultipartPartNumberV1,
    ) -> Result<ByteSize, MultipartContractError> {
        if part_number.get() > self.part_count {
            return Err(MultipartContractError::InvalidPart);
        }
        if part_number.get() < self.part_count {
            return Ok(self.part_size);
        }
        let preceding = u64::from(self.part_count - 1)
            .checked_mul(self.part_size.get())
            .ok_or(MultipartContractError::InvalidPart)?;
        ByteSize::new(
            self.total_size
                .get()
                .checked_sub(preceding)
                .ok_or(MultipartContractError::InvalidPart)?,
        )
        .map_err(|_| MultipartContractError::InvalidPart)
    }

    pub fn validate_part(
        &self,
        part_number: MultipartPartNumberV1,
        size: ByteSize,
    ) -> Result<(), MultipartContractError> {
        if self.expected_part_size(part_number)? == size {
            Ok(())
        } else {
            Err(MultipartContractError::InvalidPart)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartGrantScopeV1 {
    tenant_id: TenantId,
    key: ScopedObjectKey,
    upload_id: Option<MultipartUploadId>,
    operation: MultipartOperationV1,
}

impl MultipartGrantScopeV1 {
    pub fn new(
        tenant_id: TenantId,
        key: ScopedObjectKey,
        upload_id: Option<MultipartUploadId>,
        operation: MultipartOperationV1,
    ) -> Result<Self, MultipartContractError> {
        if key.tenant_id() != tenant_id || operation.requires_upload() != upload_id.is_some() {
            return Err(MultipartContractError::InvalidGrant);
        }
        Ok(Self {
            tenant_id,
            key,
            upload_id,
            operation,
        })
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn upload_id(&self) -> Option<MultipartUploadId> {
        self.upload_id
    }

    #[must_use]
    pub const fn operation(&self) -> MultipartOperationV1 {
        self.operation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartGrantRecordV1 {
    id: MultipartGrantId,
    digest: SecretDigest,
    key_version: MultipartGrantKeyVersion,
    scope: MultipartGrantScopeV1,
    issued_at: TimestampMillis,
    expires_at: TimestampMillis,
    revoked_at: Option<TimestampMillis>,
}

impl MultipartGrantRecordV1 {
    pub fn active(
        id: MultipartGrantId,
        digest: SecretDigest,
        key_version: MultipartGrantKeyVersion,
        scope: MultipartGrantScopeV1,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
    ) -> Result<Self, MultipartContractError> {
        if expires_at <= issued_at {
            return Err(MultipartContractError::InvalidGrant);
        }
        Ok(Self {
            id,
            digest,
            key_version,
            scope,
            issued_at,
            expires_at,
            revoked_at: None,
        })
    }

    #[must_use]
    pub const fn id(&self) -> MultipartGrantId {
        self.id
    }

    #[must_use]
    pub const fn digest(&self) -> &SecretDigest {
        &self.digest
    }

    #[must_use]
    pub const fn key_version(&self) -> MultipartGrantKeyVersion {
        self.key_version
    }

    #[must_use]
    pub const fn scope(&self) -> &MultipartGrantScopeV1 {
        &self.scope
    }

    #[must_use]
    pub const fn issued_at(&self) -> TimestampMillis {
        self.issued_at
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    #[must_use]
    pub const fn revoked_at(&self) -> Option<TimestampMillis> {
        self.revoked_at
    }

    pub fn revoke(&mut self, revoked_at: TimestampMillis) -> Result<(), MultipartContractError> {
        if revoked_at < self.issued_at {
            return Err(MultipartContractError::InvalidGrant);
        }
        self.revoked_at = Some(
            self.revoked_at
                .map_or(revoked_at, |current| current.min(revoked_at)),
        );
        Ok(())
    }

    #[must_use]
    pub fn active_at(&self, now: TimestampMillis) -> bool {
        self.issued_at <= now && now < self.expires_at && self.revoked_at.is_none_or(|at| now < at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaContainerV1 {
    Webm,
    Mp4,
    QuickTime,
    Matroska,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodecV1 {
    H264,
    H265,
    Vp8,
    Vp9,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodecV1 {
    None,
    Aac,
    Opus,
}

/// Metadata emitted by a trusted server-side probe after provider completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedMediaProbeV1 {
    container: MediaContainerV1,
    video_codec: VideoCodecV1,
    audio_codec: AudioCodecV1,
    width: u16,
    height: u16,
    duration_ms: u64,
    frame_rate_millihertz: u32,
}

impl TrustedMediaProbeV1 {
    pub fn new(
        container: MediaContainerV1,
        video_codec: VideoCodecV1,
        audio_codec: AudioCodecV1,
        width: u16,
        height: u16,
        duration_ms: u64,
        frame_rate_millihertz: u32,
    ) -> Result<Self, MultipartContractError> {
        if width == 0
            || height == 0
            || width > 32_768
            || height > 32_768
            || duration_ms == 0
            || duration_ms > crate::MAX_WIRE_INTEGER
            || !(1..=1_000_000).contains(&frame_rate_millihertz)
        {
            return Err(MultipartContractError::InvalidMediaProbe);
        }
        Ok(Self {
            container,
            video_codec,
            audio_codec,
            width,
            height,
            duration_ms,
            frame_rate_millihertz,
        })
    }

    #[must_use]
    pub const fn container(&self) -> MediaContainerV1 {
        self.container
    }

    #[must_use]
    pub const fn video_codec(&self) -> VideoCodecV1 {
        self.video_codec
    }

    #[must_use]
    pub const fn audio_codec(&self) -> AudioCodecV1 {
        self.audio_codec
    }

    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    #[must_use]
    pub const fn frame_rate_millihertz(&self) -> u32 {
        self.frame_rate_millihertz
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CorsOriginV1(String);

impl CorsOriginV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, MultipartContractError> {
        let value = value.into();
        let authority = value
            .strip_prefix("https://")
            .ok_or(MultipartContractError::InvalidCorsOrigin)?;
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) if !port.contains(':') => (host, Some(port)),
            Some(_) => return Err(MultipartContractError::InvalidCorsOrigin),
            None => (authority, None),
        };
        let host_is_canonical = !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                (1..=63).contains(&label.len())
                    && label
                        .as_bytes()
                        .first()
                        .zip(label.as_bytes().last())
                        .is_some_and(|(first, last)| {
                            first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric()
                        })
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            });
        let port_is_canonical = port.is_none_or(|port| {
            port.parse::<u16>()
                .ok()
                .is_some_and(|parsed| parsed != 0 && parsed != 443 && parsed.to_string() == port)
        });
        if authority.is_empty() || authority.len() > 253 || !host_is_canonical || !port_is_canonical
        {
            return Err(MultipartContractError::InvalidCorsOrigin);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CorsOriginV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CorsOriginV1")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for CorsOriginV1 {
    type Error = MultipartContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CorsOriginV1> for String {
    fn from(value: CorsOriginV1) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadDispositionV1 {
    Inline,
    Attachment,
}

#[cfg(test)]
mod tests {
    use crate::{ObjectRevision, StorageFileExtension, VideoId, VideoObjectDescriptor};

    use super::*;

    fn size(value: u64) -> ByteSize {
        ByteSize::new(value).expect("size")
    }

    fn limits() -> MultipartLimitsV1 {
        MultipartLimitsV1::new(
            size(5),
            size(10),
            4,
            size(40),
            size(10),
            DurationMillis::new(60_000).expect("ttl"),
        )
        .expect("limits")
    }

    fn key(tenant: TenantId) -> ScopedObjectKey {
        ScopedObjectKey::source(
            tenant,
            VideoId::new(),
            ObjectRevision::new(1).expect("revision"),
            VideoObjectDescriptor::Source {
                extension: StorageFileExtension::parse("webm").expect("extension"),
            },
        )
        .expect("key")
    }

    #[test]
    fn derives_exact_part_geometry_and_last_part() {
        let spec = MultipartUploadSpecV1::new(
            key(TenantId::new()),
            size(23),
            size(10),
            ChecksumSha256::parse("a".repeat(64)).expect("checksum"),
            ContentType::parse("video/webm").expect("content type"),
            limits(),
        )
        .expect("spec");
        assert_eq!(spec.part_count(), 3);
        assert_eq!(
            spec.expected_part_size(MultipartPartNumberV1::new(1).expect("part")),
            Ok(size(10))
        );
        assert_eq!(
            spec.expected_part_size(MultipartPartNumberV1::new(3).expect("part")),
            Ok(size(3))
        );
        assert!(
            spec.validate_part(MultipartPartNumberV1::new(3).expect("part"), size(5))
                .is_err()
        );
    }

    #[test]
    fn rejects_worker_provider_count_and_total_limit_violations() {
        let tenant = TenantId::new();
        let key = key(tenant);
        let checksum = ChecksumSha256::parse("c".repeat(64)).expect("checksum");
        let content_type = ContentType::parse("video/webm").expect("content type");
        let two_part_limit = MultipartLimitsV1::new(
            size(5),
            size(10),
            2,
            size(100),
            size(10),
            DurationMillis::new(60_000).expect("ttl"),
        )
        .expect("limits");
        assert!(
            MultipartUploadSpecV1::new(
                key.clone(),
                size(23),
                size(10),
                checksum.clone(),
                content_type.clone(),
                two_part_limit,
            )
            .is_err()
        );
        assert!(
            MultipartUploadSpecV1::new(
                key.clone(),
                size(41),
                size(10),
                checksum.clone(),
                content_type.clone(),
                limits(),
            )
            .is_err()
        );
        assert!(
            MultipartUploadSpecV1::new(
                key.clone(),
                size(23),
                size(4),
                checksum.clone(),
                content_type.clone(),
                limits(),
            )
            .is_err()
        );
        assert!(
            MultipartUploadSpecV1::new(key, size(23), size(11), checksum, content_type, limits(),)
                .is_err()
        );
        assert!(
            MultipartLimitsV1::new(
                size(5),
                size(11),
                10,
                size(100),
                size(10),
                DurationMillis::new(60_000).expect("ttl"),
            )
            .is_err()
        );
    }

    #[test]
    fn grant_material_and_object_paths_are_redacted() {
        let material = MultipartGrantSecret::parse("s".repeat(32)).expect("secret");
        assert_eq!(format!("{material:?}"), "MultipartGrantSecret([redacted])");
        assert_eq!(material.to_string(), "[redacted]");
        let scoped = key(TenantId::new());
        assert!(!format!("{scoped:?}").contains(scoped.as_str()));
    }

    #[test]
    fn grants_require_exact_upload_shape_and_have_half_open_expiry() {
        let tenant = TenantId::new();
        let key = key(tenant);
        assert!(
            MultipartGrantScopeV1::new(tenant, key.clone(), None, MultipartOperationV1::PutPart,)
                .is_err()
        );
        let scope = MultipartGrantScopeV1::new(tenant, key, None, MultipartOperationV1::Create)
            .expect("scope");
        let record = MultipartGrantRecordV1::active(
            MultipartGrantId::new(),
            SecretDigest::parse_sha256("b".repeat(64)).expect("digest"),
            MultipartGrantKeyVersion::new(1).expect("version"),
            scope,
            TimestampMillis::new(10).expect("time"),
            TimestampMillis::new(20).expect("time"),
        )
        .expect("record");
        assert!(record.active_at(TimestampMillis::new(10).expect("time")));
        assert!(!record.active_at(TimestampMillis::new(20).expect("time")));
    }

    #[test]
    fn cors_origins_are_exact_https_origins() {
        assert!(CorsOriginV1::parse("https://app.example.com").is_ok());
        assert!(CorsOriginV1::parse("https://app.example.com:8443").is_ok());
        assert!(CorsOriginV1::parse("http://app.example.com").is_err());
        assert!(CorsOriginV1::parse("https://app.example.com/path").is_err());
        assert!(CorsOriginV1::parse("https://USER@app.example.com").is_err());
        assert!(CorsOriginV1::parse("https://:").is_err());
        assert!(CorsOriginV1::parse("https://-bad.example").is_err());
        assert!(CorsOriginV1::parse("https://bad-.example").is_err());
        assert!(CorsOriginV1::parse("https://app.example.com:0").is_err());
        assert!(CorsOriginV1::parse("https://app.example.com:0443").is_err());
        assert!(CorsOriginV1::parse("https://app.example.com:443").is_err());
    }
}
