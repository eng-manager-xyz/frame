use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ByteSize, ChecksumSha256, ContentType, MediaExecutorKind, MediaProfileVersion, ObjectKey,
    ObjectRole, TenantId, TimestampMillis, VideoId,
};

pub const STORAGE_KEY_SCHEMA_VERSION: u16 = 1;
pub const DERIVATIVE_MANIFEST_SCHEMA_VERSION: u16 = 1;
const MAX_OBJECT_REVISION: u64 = 9_007_199_254_740_991;
const MAX_SEGMENT_INDEX: u32 = 99_999_999;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum StorageContractError {
    #[error("object revision is invalid")]
    InvalidRevision,
    #[error("object file extension is invalid")]
    InvalidFileExtension,
    #[error("recording segment index is invalid")]
    InvalidSegmentIndex,
    #[error("object role is invalid for this key layout")]
    InvalidObjectRole,
    #[error("transform profile name is invalid")]
    InvalidProfileName,
    #[error("transform profile is not in canonical normalized form")]
    InvalidNormalizedProfile,
    #[error("storage object key is invalid or non-canonical")]
    InvalidObjectKey,
    #[error("derivative manifest is inconsistent")]
    InvalidDerivativeManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct ObjectRevision(u64);

impl ObjectRevision {
    pub fn new(value: u64) -> Result<Self, StorageContractError> {
        if value == 0 || value > MAX_OBJECT_REVISION {
            return Err(StorageContractError::InvalidRevision);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for ObjectRevision {
    type Error = StorageContractError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ObjectRevision> for u64 {
    fn from(value: ObjectRevision) -> Self {
        value.0
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StorageFileExtension(String);

impl StorageFileExtension {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageContractError> {
        let value = value.into();
        if !(1..=16).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err(StorageContractError::InvalidFileExtension);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for StorageFileExtension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StorageFileExtension")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for StorageFileExtension {
    type Error = StorageContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<StorageFileExtension> for String {
    fn from(value: StorageFileExtension) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct RecordingSegmentIndex(u32);

impl RecordingSegmentIndex {
    pub fn new(value: u32) -> Result<Self, StorageContractError> {
        if value > MAX_SEGMENT_INDEX {
            return Err(StorageContractError::InvalidSegmentIndex);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for RecordingSegmentIndex {
    type Error = StorageContractError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RecordingSegmentIndex> for u32 {
    fn from(value: RecordingSegmentIndex) -> Self {
        value.0
    }
}

/// A closed set of generated names. No variant accepts a user-provided basename.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoObjectDescriptor {
    Source {
        extension: StorageFileExtension,
    },
    RecordingSegment {
        index: RecordingSegmentIndex,
        extension: StorageFileExtension,
    },
    Thumbnail {
        extension: StorageFileExtension,
    },
    Preview {
        extension: StorageFileExtension,
    },
    Spritesheet {
        extension: StorageFileExtension,
    },
    Audio {
        extension: StorageFileExtension,
    },
    Export {
        extension: StorageFileExtension,
    },
    Manifest,
}

impl VideoObjectDescriptor {
    #[must_use]
    pub const fn role(&self) -> ObjectRole {
        match self {
            Self::Source { .. } => ObjectRole::Source,
            Self::RecordingSegment { .. } => ObjectRole::RecordingSegment,
            Self::Thumbnail { .. } => ObjectRole::Thumbnail,
            Self::Preview { .. } => ObjectRole::Preview,
            Self::Spritesheet { .. } => ObjectRole::Spritesheet,
            Self::Audio { .. } => ObjectRole::Audio,
            Self::Export { .. } => ObjectRole::Export,
            Self::Manifest => ObjectRole::Manifest,
        }
    }

    #[must_use]
    pub const fn is_source(&self) -> bool {
        matches!(
            Self::role(self),
            ObjectRole::Source | ObjectRole::RecordingSegment
        )
    }

    #[must_use]
    pub const fn is_derivative(&self) -> bool {
        matches!(
            Self::role(self),
            ObjectRole::Thumbnail
                | ObjectRole::Preview
                | ObjectRole::Spritesheet
                | ObjectRole::Audio
                | ObjectRole::Export
        )
    }

    #[must_use]
    pub const fn is_manifest(&self) -> bool {
        matches!(self, Self::Manifest)
    }

    #[must_use]
    pub const fn is_profile_artifact(&self) -> bool {
        Self::is_derivative(self) || Self::is_manifest(self)
    }

    fn file_name(&self) -> String {
        match self {
            Self::Source { extension } => format!("source.{}", extension.as_str()),
            Self::RecordingSegment { index, extension } => {
                format!("segment-{:08}.{}", index.get(), extension.as_str())
            }
            Self::Thumbnail { extension } => format!("thumbnail.{}", extension.as_str()),
            Self::Preview { extension } => format!("preview.{}", extension.as_str()),
            Self::Spritesheet { extension } => format!("spritesheet.{}", extension.as_str()),
            Self::Audio { extension } => format!("audio.{}", extension.as_str()),
            Self::Export { extension } => format!("export.{}", extension.as_str()),
            Self::Manifest => "manifest.json".to_owned(),
        }
    }

    fn parse(role: ObjectRole, value: &str) -> Result<Self, StorageContractError> {
        if role == ObjectRole::Manifest {
            return (value == "manifest.json")
                .then_some(Self::Manifest)
                .ok_or(StorageContractError::InvalidObjectKey);
        }
        let (stem, extension) = value
            .split_once('.')
            .ok_or(StorageContractError::InvalidObjectKey)?;
        if extension.contains('.') {
            return Err(StorageContractError::InvalidObjectKey);
        }
        let extension = StorageFileExtension::parse(extension)
            .map_err(|_| StorageContractError::InvalidObjectKey)?;
        match role {
            ObjectRole::Source if stem == "source" => Ok(Self::Source { extension }),
            ObjectRole::RecordingSegment => {
                let digits = stem
                    .strip_prefix("segment-")
                    .ok_or(StorageContractError::InvalidObjectKey)?;
                if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(StorageContractError::InvalidObjectKey);
                }
                let index = digits
                    .parse::<u32>()
                    .ok()
                    .and_then(|value| RecordingSegmentIndex::new(value).ok())
                    .ok_or(StorageContractError::InvalidObjectKey)?;
                Ok(Self::RecordingSegment { index, extension })
            }
            ObjectRole::Thumbnail if stem == "thumbnail" => Ok(Self::Thumbnail { extension }),
            ObjectRole::Preview if stem == "preview" => Ok(Self::Preview { extension }),
            ObjectRole::Spritesheet if stem == "spritesheet" => Ok(Self::Spritesheet { extension }),
            ObjectRole::Audio if stem == "audio" => Ok(Self::Audio { extension }),
            ObjectRole::Export if stem == "export" => Ok(Self::Export { extension }),
            _ => Err(StorageContractError::InvalidObjectKey),
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TransformProfileName(String);

impl TransformProfileName {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageContractError> {
        let value = value.into();
        let valid_edge = value
            .as_bytes()
            .first()
            .zip(value.as_bytes().last())
            .is_some_and(|(first, last)| {
                first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric()
            });
        if !(1..=48).contains(&value.len())
            || !valid_edge
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(StorageContractError::InvalidProfileName);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TransformProfileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TransformProfileName")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for TransformProfileName {
    type Error = StorageContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<TransformProfileName> for String {
    fn from(value: TransformProfileName) -> Self {
        value.0
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NormalizedTransformProfile(String);

impl NormalizedTransformProfile {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageContractError> {
        let value = value.into();
        if value.is_empty() || value.len() > 1_024 || !profile_pairs_are_canonical(&value) {
            return Err(StorageContractError::InvalidNormalizedProfile);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NormalizedTransformProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizedTransformProfile")
            .field("byte_length", &self.0.len())
            .finish_non_exhaustive()
    }
}

impl TryFrom<String> for NormalizedTransformProfile {
    type Error = StorageContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<NormalizedTransformProfile> for String {
    fn from(value: NormalizedTransformProfile) -> Self {
        value.0
    }
}

fn profile_pairs_are_canonical(value: &str) -> bool {
    let mut previous_key: Option<&str> = None;
    value.split(';').all(|pair| {
        let Some((key, value)) = pair.split_once('=') else {
            return false;
        };
        let key_valid = (1..=32).contains(&key.len())
            && key.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && key.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            });
        let value_valid = (1..=128).contains(&value.len())
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_' | b':' | b'+' | b'/' | b',')
            });
        let ordered = previous_key.is_none_or(|previous| previous < key);
        previous_key = Some(key);
        key_valid && value_valid && ordered
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TransformProfile {
    name: TransformProfileName,
    version: MediaProfileVersion,
    normalized: NormalizedTransformProfile,
    output_descriptor: VideoObjectDescriptor,
    fingerprint: ChecksumSha256,
}

impl TransformProfile {
    pub fn new(
        name: TransformProfileName,
        version: MediaProfileVersion,
        normalized: NormalizedTransformProfile,
        output_descriptor: VideoObjectDescriptor,
    ) -> Result<Self, StorageContractError> {
        if !output_descriptor.is_derivative() {
            return Err(StorageContractError::InvalidObjectRole);
        }
        let fingerprint = profile_fingerprint(&name, &normalized, &output_descriptor);
        Ok(Self {
            name,
            version,
            normalized,
            output_descriptor,
            fingerprint,
        })
    }

    #[must_use]
    pub const fn name(&self) -> &TransformProfileName {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> MediaProfileVersion {
        self.version
    }

    #[must_use]
    pub const fn normalized(&self) -> &NormalizedTransformProfile {
        &self.normalized
    }

    #[must_use]
    pub const fn output_descriptor(&self) -> &VideoObjectDescriptor {
        &self.output_descriptor
    }

    #[must_use]
    pub const fn fingerprint(&self) -> &ChecksumSha256 {
        &self.fingerprint
    }

    fn key_reference(&self) -> DerivativeProfileReference {
        DerivativeProfileReference {
            version: self.version,
            fingerprint: self.fingerprint.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformProfileWire {
    name: TransformProfileName,
    version: MediaProfileVersion,
    normalized: NormalizedTransformProfile,
    output_descriptor: VideoObjectDescriptor,
    fingerprint: ChecksumSha256,
}

impl<'de> Deserialize<'de> for TransformProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TransformProfileWire::deserialize(deserializer)?;
        if wire.version.get() == 0 {
            return Err(de::Error::custom("transform profile version is invalid"));
        }
        let profile = Self::new(
            wire.name,
            wire.version,
            wire.normalized,
            wire.output_descriptor,
        )
        .map_err(de::Error::custom)?;
        if profile.fingerprint != wire.fingerprint {
            return Err(de::Error::custom("transform profile fingerprint mismatch"));
        }
        Ok(profile)
    }
}

fn profile_fingerprint(
    name: &TransformProfileName,
    normalized: &NormalizedTransformProfile,
    output_descriptor: &VideoObjectDescriptor,
) -> ChecksumSha256 {
    let output_role = output_descriptor.role().path_segment();
    let output_name = output_descriptor.file_name();
    let mut framed = Vec::with_capacity(
        name.as_str().len()
            + normalized.as_str().len()
            + output_role.len()
            + output_name.len()
            + 32,
    );
    update_length_framed(&mut framed, name.as_str().as_bytes());
    update_length_framed(&mut framed, normalized.as_str().as_bytes());
    update_length_framed(&mut framed, output_role.as_bytes());
    update_length_framed(&mut framed, output_name.as_bytes());
    ChecksumSha256::digest_bytes(&framed)
}

fn update_length_framed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    output.extend_from_slice(value);
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct DerivativeProfileReference {
    version: MediaProfileVersion,
    fingerprint: ChecksumSha256,
}

impl DerivativeProfileReference {
    fn token(&self) -> String {
        format!("p{}-{}", self.version.get(), self.fingerprint.as_str())
    }

    fn parse(value: &str) -> Result<Self, StorageContractError> {
        let value = value
            .strip_prefix('p')
            .ok_or(StorageContractError::InvalidObjectKey)?;
        let (version, fingerprint) = value
            .split_once('-')
            .ok_or(StorageContractError::InvalidObjectKey)?;
        Ok(Self {
            version: MediaProfileVersion::new(
                version
                    .parse::<u16>()
                    .map_err(|_| StorageContractError::InvalidObjectKey)?,
            )
            .map_err(|_| StorageContractError::InvalidObjectKey)?,
            fingerprint: ChecksumSha256::parse(fingerprint)
                .map_err(|_| StorageContractError::InvalidObjectKey)?,
        })
    }

    fn matches(&self, profile: &TransformProfile) -> bool {
        self.version == profile.version && self.fingerprint == profile.fingerprint
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ScopedObjectKey {
    tenant_id: TenantId,
    video_id: VideoId,
    source_revision: ObjectRevision,
    descriptor: VideoObjectDescriptor,
    profile: Option<DerivativeProfileReference>,
    key: ObjectKey,
}

impl ScopedObjectKey {
    pub fn source(
        tenant_id: TenantId,
        video_id: VideoId,
        revision: ObjectRevision,
        descriptor: VideoObjectDescriptor,
    ) -> Result<Self, StorageContractError> {
        if !descriptor.is_source() {
            return Err(StorageContractError::InvalidObjectRole);
        }
        Self::build(tenant_id, video_id, revision, descriptor, None)
    }

    pub fn derivative(
        tenant_id: TenantId,
        video_id: VideoId,
        source_revision: ObjectRevision,
        profile: &TransformProfile,
    ) -> Result<Self, StorageContractError> {
        Self::build(
            tenant_id,
            video_id,
            source_revision,
            profile.output_descriptor.clone(),
            Some(profile.key_reference()),
        )
    }

    pub fn manifest(
        tenant_id: TenantId,
        video_id: VideoId,
        source_revision: ObjectRevision,
        profile: &TransformProfile,
    ) -> Result<Self, StorageContractError> {
        Self::build(
            tenant_id,
            video_id,
            source_revision,
            VideoObjectDescriptor::Manifest,
            Some(profile.key_reference()),
        )
    }

    fn build(
        tenant_id: TenantId,
        video_id: VideoId,
        source_revision: ObjectRevision,
        descriptor: VideoObjectDescriptor,
        profile: Option<DerivativeProfileReference>,
    ) -> Result<Self, StorageContractError> {
        let role = descriptor.role();
        let key_value = if let Some(profile) = &profile {
            format!(
                "tenants/{tenant_id}/videos/{video_id}/v{STORAGE_KEY_SCHEMA_VERSION}/{}/source-r{}/{}/{}",
                role.path_segment(),
                source_revision.get(),
                profile.token(),
                descriptor.file_name()
            )
        } else {
            format!(
                "tenants/{tenant_id}/videos/{video_id}/v{STORAGE_KEY_SCHEMA_VERSION}/{}/r{}/{}",
                role.path_segment(),
                source_revision.get(),
                descriptor.file_name()
            )
        };
        let key =
            ObjectKey::parse(key_value).map_err(|_| StorageContractError::InvalidObjectKey)?;
        Ok(Self {
            tenant_id,
            video_id,
            source_revision,
            descriptor,
            profile,
            key,
        })
    }

    pub fn parse(value: &str) -> Result<Self, StorageContractError> {
        let segments = value.split('/').collect::<Vec<_>>();
        if segments.len() != 8 && segments.len() != 9 {
            return Err(StorageContractError::InvalidObjectKey);
        }
        if segments[0] != "tenants"
            || segments[2] != "videos"
            || segments[4] != format!("v{STORAGE_KEY_SCHEMA_VERSION}")
        {
            return Err(StorageContractError::InvalidObjectKey);
        }
        let tenant_id =
            TenantId::parse(segments[1]).map_err(|_| StorageContractError::InvalidObjectKey)?;
        let video_id = VideoId::parse_strict(segments[3])
            .map_err(|_| StorageContractError::InvalidObjectKey)?;
        let role = role_from_segment(segments[5])?;
        let parsed = if segments.len() == 8 {
            let revision = parse_prefixed_revision(segments[6], 'r')?;
            let descriptor = VideoObjectDescriptor::parse(role, segments[7])?;
            Self::source(tenant_id, video_id, revision, descriptor)?
        } else {
            let revision_value = segments[6]
                .strip_prefix("source-r")
                .ok_or(StorageContractError::InvalidObjectKey)?;
            let revision = ObjectRevision::new(
                revision_value
                    .parse::<u64>()
                    .map_err(|_| StorageContractError::InvalidObjectKey)?,
            )
            .map_err(|_| StorageContractError::InvalidObjectKey)?;
            let profile = DerivativeProfileReference::parse(segments[7])?;
            let descriptor = VideoObjectDescriptor::parse(role, segments[8])?;
            if !descriptor.is_profile_artifact() {
                return Err(StorageContractError::InvalidObjectKey);
            }
            Self::build(tenant_id, video_id, revision, descriptor, Some(profile))?
        };
        if parsed.as_str() != value {
            return Err(StorageContractError::InvalidObjectKey);
        }
        Ok(parsed)
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn video_id(&self) -> VideoId {
        self.video_id
    }

    #[must_use]
    pub const fn source_revision(&self) -> ObjectRevision {
        self.source_revision
    }

    #[must_use]
    pub const fn descriptor(&self) -> &VideoObjectDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn role(&self) -> ObjectRole {
        self.descriptor.role()
    }

    #[must_use]
    pub const fn is_source(&self) -> bool {
        self.profile.is_none()
    }

    #[must_use]
    pub const fn is_derivative(&self) -> bool {
        self.profile.is_some() && self.descriptor.is_derivative()
    }

    #[must_use]
    pub const fn is_manifest(&self) -> bool {
        self.profile.is_some() && self.descriptor.is_manifest()
    }

    #[must_use]
    pub fn profile_matches(&self, profile: &TransformProfile) -> bool {
        self.profile
            .as_ref()
            .is_some_and(|reference| reference.matches(profile))
            && (self.descriptor == profile.output_descriptor || self.descriptor.is_manifest())
    }

    #[must_use]
    pub fn belongs_to(&self, tenant_id: TenantId, video_id: VideoId) -> bool {
        self.tenant_id == tenant_id && self.video_id == video_id
    }

    #[must_use]
    pub fn as_object_key(&self) -> &ObjectKey {
        &self.key
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.key.as_str()
    }
}

impl fmt::Debug for ScopedObjectKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedObjectKey")
            .field("role", &self.role())
            .field("source_revision", &self.source_revision)
            .field("value", &"[redacted]")
            .finish()
    }
}

impl FromStr for ScopedObjectKey {
    type Err = StorageContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for ScopedObjectKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ScopedObjectKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

fn parse_prefixed_revision(
    value: &str,
    prefix: char,
) -> Result<ObjectRevision, StorageContractError> {
    let value = value
        .strip_prefix(prefix)
        .ok_or(StorageContractError::InvalidObjectKey)?;
    ObjectRevision::new(
        value
            .parse::<u64>()
            .map_err(|_| StorageContractError::InvalidObjectKey)?,
    )
    .map_err(|_| StorageContractError::InvalidObjectKey)
}

fn role_from_segment(value: &str) -> Result<ObjectRole, StorageContractError> {
    match value {
        "source" => Ok(ObjectRole::Source),
        "segment" => Ok(ObjectRole::RecordingSegment),
        "thumbnail" => Ok(ObjectRole::Thumbnail),
        "preview" => Ok(ObjectRole::Preview),
        "spritesheet" => Ok(ObjectRole::Spritesheet),
        "audio" => Ok(ObjectRole::Audio),
        "export" => Ok(ObjectRole::Export),
        "manifest" => Ok(ObjectRole::Manifest),
        _ => Err(StorageContractError::InvalidObjectKey),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct DerivativeAttempt(u32);

impl DerivativeAttempt {
    pub fn new(value: u32) -> Result<Self, StorageContractError> {
        if value == 0 {
            return Err(StorageContractError::InvalidDerivativeManifest);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for DerivativeAttempt {
    type Error = StorageContractError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DerivativeAttempt> for u32 {
    fn from(value: DerivativeAttempt) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DerivativeManifest {
    schema_version: u16,
    source: ScopedObjectKey,
    source_checksum: ChecksumSha256,
    profile: TransformProfile,
    executor: MediaExecutorKind,
    output: ScopedObjectKey,
    output_checksum: ChecksumSha256,
    output_content_type: ContentType,
    output_size: ByteSize,
    attempt: DerivativeAttempt,
    created_at: TimestampMillis,
}

impl DerivativeManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: ScopedObjectKey,
        source_checksum: ChecksumSha256,
        profile: TransformProfile,
        executor: MediaExecutorKind,
        output: ScopedObjectKey,
        output_checksum: ChecksumSha256,
        output_content_type: ContentType,
        output_size: ByteSize,
        attempt: DerivativeAttempt,
        created_at: TimestampMillis,
    ) -> Result<Self, StorageContractError> {
        if !source.is_source()
            || !output.is_derivative()
            || source.tenant_id != output.tenant_id
            || source.video_id != output.video_id
            || source.source_revision != output.source_revision
            || !output.profile_matches(&profile)
            || !executor.dispatchable()
            || output_size.get() == 0
            || ChecksumSha256::parse(source_checksum.as_str()).is_err()
            || ChecksumSha256::parse(output_checksum.as_str()).is_err()
            || ContentType::parse(output_content_type.as_str()).is_err()
            || ByteSize::new(output_size.get()).is_err()
            || TimestampMillis::new(created_at.get()).is_err()
        {
            return Err(StorageContractError::InvalidDerivativeManifest);
        }
        Ok(Self {
            schema_version: DERIVATIVE_MANIFEST_SCHEMA_VERSION,
            source,
            source_checksum,
            profile,
            executor,
            output,
            output_checksum,
            output_content_type,
            output_size,
            attempt,
            created_at,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn source(&self) -> &ScopedObjectKey {
        &self.source
    }

    #[must_use]
    pub const fn source_checksum(&self) -> &ChecksumSha256 {
        &self.source_checksum
    }

    #[must_use]
    pub const fn profile(&self) -> &TransformProfile {
        &self.profile
    }

    #[must_use]
    pub const fn executor(&self) -> MediaExecutorKind {
        self.executor
    }

    #[must_use]
    pub const fn output(&self) -> &ScopedObjectKey {
        &self.output
    }

    #[must_use]
    pub const fn output_checksum(&self) -> &ChecksumSha256 {
        &self.output_checksum
    }

    #[must_use]
    pub const fn output_content_type(&self) -> &ContentType {
        &self.output_content_type
    }

    #[must_use]
    pub const fn output_size(&self) -> ByteSize {
        self.output_size
    }

    #[must_use]
    pub const fn attempt(&self) -> DerivativeAttempt {
        self.attempt
    }

    #[must_use]
    pub const fn created_at(&self) -> TimestampMillis {
        self.created_at
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DerivativeManifestWire {
    schema_version: u16,
    source: ScopedObjectKey,
    source_checksum: ChecksumSha256,
    profile: TransformProfile,
    executor: MediaExecutorKind,
    output: ScopedObjectKey,
    output_checksum: ChecksumSha256,
    output_content_type: ContentType,
    output_size: ByteSize,
    attempt: DerivativeAttempt,
    created_at: TimestampMillis,
}

impl<'de> Deserialize<'de> for DerivativeManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DerivativeManifestWire::deserialize(deserializer)?;
        if wire.schema_version != DERIVATIVE_MANIFEST_SCHEMA_VERSION {
            return Err(de::Error::custom("unsupported derivative manifest version"));
        }
        Self::new(
            wire.source,
            wire.source_checksum,
            wire.profile,
            wire.executor,
            wire.output,
            wire.output_checksum,
            wire.output_content_type,
            wire.output_size,
            wire.attempt,
            wire.created_at,
        )
        .map_err(de::Error::custom)
    }
}

impl ChecksumSha256 {
    #[must_use]
    pub fn digest_bytes(bytes: &[u8]) -> Self {
        Self::parse(hex_sha256(Sha256::digest(bytes).as_slice()))
            .expect("SHA-256 is always 64 lowercase hexadecimal characters")
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension(value: &str) -> StorageFileExtension {
        StorageFileExtension::parse(value).expect("valid extension")
    }

    fn profile(value: &str) -> TransformProfile {
        TransformProfile::new(
            TransformProfileName::parse("web-preview").expect("name"),
            MediaProfileVersion::new(3).expect("version"),
            NormalizedTransformProfile::parse(value).expect("profile"),
            VideoObjectDescriptor::Preview {
                extension: extension("webm"),
            },
        )
        .expect("transform profile")
    }

    fn thumbnail_profile(value: &str) -> TransformProfile {
        TransformProfile::new(
            TransformProfileName::parse("web-thumbnail").expect("name"),
            MediaProfileVersion::new(3).expect("version"),
            NormalizedTransformProfile::parse(value).expect("profile"),
            VideoObjectDescriptor::Thumbnail {
                extension: extension("webp"),
            },
        )
        .expect("transform profile")
    }

    #[test]
    fn source_and_derivative_keys_are_canonical_and_round_trip() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let revision = ObjectRevision::new(7).expect("revision");
        let source = ScopedObjectKey::source(
            tenant,
            video,
            revision,
            VideoObjectDescriptor::Source {
                extension: extension("webm"),
            },
        )
        .expect("source");
        assert!(source.as_object_key().belongs_to_tenant(tenant));
        assert_eq!(ScopedObjectKey::parse(source.as_str()), Ok(source.clone()));
        let output = ScopedObjectKey::derivative(
            tenant,
            video,
            revision,
            &thumbnail_profile("fit=cover;height=720;width=1280"),
        )
        .expect("derivative");
        assert_eq!(ScopedObjectKey::parse(output.as_str()), Ok(output));
    }

    #[test]
    fn filenames_cannot_contain_user_basename_unicode_or_traversal() {
        for invalid in [
            "WEBM",
            "web.m",
            "../webm",
            "webm/secret",
            "résumé",
            "reallylongextension",
            "",
        ] {
            assert!(StorageFileExtension::parse(invalid).is_err(), "{invalid}");
        }
        let tenant = TenantId::new();
        let video = VideoId::new();
        for invalid in [
            format!("tenants/{tenant}/videos/{video}/v1/source/r1/quarterly-plan.webm"),
            format!("tenants/{tenant}/videos/{video}/v1/source/r1/../source.webm"),
            format!("tenants/{tenant}/videos/{video}/v1/source/r01/source.webm"),
            format!("tenants/{tenant}/videos/{video}/v1/source/r1/source.WEBM"),
        ] {
            assert!(ScopedObjectKey::parse(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn normalized_profiles_reject_reordering_duplicates_and_ambiguous_values() {
        assert!(NormalizedTransformProfile::parse("height=720;width=1280").is_ok());
        for invalid in [
            "width=1280;height=720",
            "height=720;height=1080",
            "height =720",
            "height=720;",
            "Height=720",
            "height=720p@30",
        ] {
            assert!(
                NormalizedTransformProfile::parse(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn profile_or_source_revision_changes_derivative_key() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let first = ScopedObjectKey::derivative(
            tenant,
            video,
            ObjectRevision::new(1).expect("revision"),
            &profile("height=720;width=1280"),
        )
        .expect("first");
        let changed_revision = ScopedObjectKey::derivative(
            tenant,
            video,
            ObjectRevision::new(2).expect("revision"),
            &profile("height=720;width=1280"),
        )
        .expect("revision");
        let changed_profile = ScopedObjectKey::derivative(
            tenant,
            video,
            ObjectRevision::new(1).expect("revision"),
            &profile("height=1080;width=1920"),
        )
        .expect("profile");
        assert_ne!(first, changed_revision);
        assert_ne!(first, changed_profile);
    }

    #[test]
    fn profile_version_is_a_distinct_derivative_identity() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let normalized =
            NormalizedTransformProfile::parse("height=720;width=1280").expect("profile");
        let descriptor = VideoObjectDescriptor::Preview {
            extension: extension("webm"),
        };
        let version_one = TransformProfile::new(
            TransformProfileName::parse("web-preview").expect("name"),
            MediaProfileVersion::new(1).expect("version"),
            normalized.clone(),
            descriptor.clone(),
        )
        .expect("profile");
        let version_two = TransformProfile::new(
            TransformProfileName::parse("web-preview").expect("name"),
            MediaProfileVersion::new(2).expect("version"),
            normalized,
            descriptor,
        )
        .expect("profile");
        assert_eq!(version_one.fingerprint(), version_two.fingerprint());
        assert_ne!(
            ScopedObjectKey::derivative(
                tenant,
                video,
                ObjectRevision::new(1).expect("revision"),
                &version_one,
            )
            .expect("key"),
            ScopedObjectKey::derivative(
                tenant,
                video,
                ObjectRevision::new(1).expect("revision"),
                &version_two,
            )
            .expect("key"),
        );
    }

    #[test]
    fn profile_fingerprint_is_framed_stable_and_boundary_checked() {
        let fixed = profile("height=720;width=1280");
        assert_eq!(
            fixed.fingerprint().as_str(),
            "ec099737d8deb5c5ea18b5503e55540212142ed9ba78873c063d2f56af13b60f"
        );

        let first = TransformProfile::new(
            TransformProfileName::parse("a").expect("name"),
            MediaProfileVersion::new(1).expect("version"),
            NormalizedTransformProfile::parse("bc=d").expect("profile"),
            VideoObjectDescriptor::Preview {
                extension: extension("webm"),
            },
        )
        .expect("profile");
        let second = TransformProfile::new(
            TransformProfileName::parse("ab").expect("name"),
            MediaProfileVersion::new(1).expect("version"),
            NormalizedTransformProfile::parse("c=d").expect("profile"),
            VideoObjectDescriptor::Preview {
                extension: extension("webm"),
            },
        )
        .expect("profile");
        assert_eq!(
            format!("{}{}", first.name().as_str(), first.normalized().as_str()),
            format!("{}{}", second.name().as_str(), second.normalized().as_str())
        );
        assert_ne!(first.fingerprint(), second.fingerprint());

        assert!(TransformProfileName::parse("a").is_ok());
        assert!(TransformProfileName::parse("a".repeat(48)).is_ok());
        assert!(TransformProfileName::parse("a".repeat(49)).is_err());
        assert!(
            NormalizedTransformProfile::parse(format!("{}={}", "a".repeat(32), "b".repeat(128)))
                .is_ok()
        );
        assert!(NormalizedTransformProfile::parse(format!("{}=b", "a".repeat(33))).is_err());
        assert!(NormalizedTransformProfile::parse(format!("a={}", "b".repeat(129))).is_err());
    }

    #[test]
    fn transform_profile_binds_one_media_output_and_manifest_is_not_an_output() {
        let name = || TransformProfileName::parse("invalid-output").expect("name");
        let normalized = || NormalizedTransformProfile::parse("height=720").expect("profile");
        assert_eq!(
            TransformProfile::new(
                name(),
                MediaProfileVersion::new(1).expect("version"),
                normalized(),
                VideoObjectDescriptor::Manifest,
            ),
            Err(StorageContractError::InvalidObjectRole)
        );
        assert_eq!(
            TransformProfile::new(
                name(),
                MediaProfileVersion::new(1).expect("version"),
                normalized(),
                VideoObjectDescriptor::Source {
                    extension: extension("webm"),
                },
            ),
            Err(StorageContractError::InvalidObjectRole)
        );

        let tenant = TenantId::new();
        let video = VideoId::new();
        let revision = ObjectRevision::new(1).expect("revision");
        let transform = profile("height=720;width=1280");
        let output =
            ScopedObjectKey::derivative(tenant, video, revision, &transform).expect("derivative");
        assert_eq!(output.descriptor(), transform.output_descriptor());
        let manifest =
            ScopedObjectKey::manifest(tenant, video, revision, &transform).expect("manifest key");
        assert!(manifest.is_manifest());
        assert!(!manifest.is_derivative());
        assert_eq!(
            ScopedObjectKey::parse(manifest.as_str()),
            Ok(manifest.clone())
        );

        let source = ScopedObjectKey::source(
            tenant,
            video,
            revision,
            VideoObjectDescriptor::Source {
                extension: extension("webm"),
            },
        )
        .expect("source");
        let checksum = ChecksumSha256::digest_bytes(b"content");
        assert_eq!(
            DerivativeManifest::new(
                source,
                checksum.clone(),
                transform,
                MediaExecutorKind::NativeGstreamer,
                manifest,
                checksum.clone(),
                ContentType::parse("application/json").expect("content type"),
                ByteSize::new(7).expect("size"),
                DerivativeAttempt::new(1).expect("attempt"),
                TimestampMillis::new(1).expect("timestamp"),
            ),
            Err(StorageContractError::InvalidDerivativeManifest)
        );
    }

    #[test]
    fn derivative_manifest_rejects_cross_scope_and_provenance_drift() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let revision = ObjectRevision::new(1).expect("revision");
        let source = ScopedObjectKey::source(
            tenant,
            video,
            revision,
            VideoObjectDescriptor::Source {
                extension: extension("webm"),
            },
        )
        .expect("source");
        let transform = profile("height=720;width=1280");
        let output =
            ScopedObjectKey::derivative(tenant, video, revision, &transform).expect("output");
        let checksum = ChecksumSha256::digest_bytes(b"content");
        let manifest = DerivativeManifest::new(
            source.clone(),
            checksum.clone(),
            transform.clone(),
            MediaExecutorKind::NativeGstreamer,
            output,
            checksum.clone(),
            ContentType::parse("video/webm").expect("content type"),
            ByteSize::new(7).expect("size"),
            DerivativeAttempt::new(1).expect("attempt"),
            TimestampMillis::new(1).expect("timestamp"),
        );
        assert!(manifest.is_ok());
        let other_output =
            ScopedObjectKey::derivative(TenantId::new(), video, revision, &transform)
                .expect("other output");
        assert_eq!(
            DerivativeManifest::new(
                source,
                checksum.clone(),
                transform,
                MediaExecutorKind::CloudflareMedia,
                other_output,
                checksum,
                ContentType::parse("video/webm").expect("content type"),
                ByteSize::new(7).expect("size"),
                DerivativeAttempt::new(1).expect("attempt"),
                TimestampMillis::new(1).expect("timestamp"),
            ),
            Err(StorageContractError::InvalidDerivativeManifest)
        );
    }

    #[test]
    fn sha256_matches_standard_known_answers() {
        assert_eq!(
            ChecksumSha256::digest_bytes(b"").as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            ChecksumSha256::digest_bytes(b"abc").as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            ChecksumSha256::digest_bytes(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )
            .as_str(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn profile_and_manifest_wire_formats_reject_unknown_fields() {
        let transform = profile("height=720;width=1280");
        let mut profile_value = serde_json::to_value(&transform).expect("serialize profile");
        profile_value
            .as_object_mut()
            .expect("profile object")
            .insert("future_field".to_owned(), serde_json::json!(true));
        assert!(serde_json::from_value::<TransformProfile>(profile_value).is_err());

        let tenant = TenantId::new();
        let video = VideoId::new();
        let revision = ObjectRevision::new(1).expect("revision");
        let source = ScopedObjectKey::source(
            tenant,
            video,
            revision,
            VideoObjectDescriptor::Source {
                extension: extension("webm"),
            },
        )
        .expect("source");
        let output =
            ScopedObjectKey::derivative(tenant, video, revision, &transform).expect("output");
        let checksum = ChecksumSha256::digest_bytes(b"content");
        let manifest = DerivativeManifest::new(
            source,
            checksum.clone(),
            transform,
            MediaExecutorKind::NativeGstreamer,
            output,
            checksum.clone(),
            ContentType::parse("video/webm").expect("content type"),
            ByteSize::new(7).expect("size"),
            DerivativeAttempt::new(1).expect("attempt"),
            TimestampMillis::new(1).expect("timestamp"),
        )
        .expect("manifest");
        let mut manifest_value = serde_json::to_value(&manifest).expect("serialize manifest");
        manifest_value
            .as_object_mut()
            .expect("manifest object")
            .insert("future_field".to_owned(), serde_json::json!(true));
        assert!(serde_json::from_value::<DerivativeManifest>(manifest_value).is_err());
    }
}
