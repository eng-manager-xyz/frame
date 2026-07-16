use std::{collections::BTreeMap, fmt};

use thiserror::Error;

pub const INSTANT_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantState {
    Idle,
    Recording,
    Offline,
    Recovering,
    Finalizing,
    Ready,
    Cancelled,
    Failed,
}

impl InstantState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Ready | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    Spooled,
    Uploaded,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentDescriptor {
    pub index: u32,
    pub start_ns: u64,
    pub duration_ns: u64,
    pub bytes: u64,
    pub checksum_sha256: String,
}

impl SegmentDescriptor {
    pub fn validate(self) -> Result<Self, InstantError> {
        if self.duration_ns == 0
            || self.bytes == 0
            || self.start_ns.checked_add(self.duration_ns).is_none()
        {
            return Err(InstantError::InvalidSegment);
        }
        if self.checksum_sha256.len() != 64
            || !self
                .checksum_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(InstantError::InvalidChecksum);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRecord {
    pub descriptor: SegmentDescriptor,
    pub state: SegmentState,
    pub spool_retained: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeManifest {
    pub protocol_version: u16,
    pub journal_revision: u64,
    pub segment_count: u32,
    pub total_bytes: u64,
    pub duration_ns: u64,
    pub checksums: Vec<String>,
}

#[derive(Clone)]
pub struct InstantSession {
    state: InstantState,
    journal_revision: u64,
    max_spool_bytes: u64,
    retained_spool_bytes: u64,
    segments: BTreeMap<u32, SegmentRecord>,
    recover_to: Option<InstantState>,
    published_identity: Option<String>,
}

impl fmt::Debug for InstantSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantSession")
            .field("state", &self.state)
            .field("journal_revision", &self.journal_revision)
            .field("max_spool_bytes", &self.max_spool_bytes)
            .field("retained_spool_bytes", &self.retained_spool_bytes)
            .field("segment_count", &self.segments.len())
            .field("published", &self.published_identity.is_some())
            .finish()
    }
}

impl InstantSession {
    pub fn new(max_spool_bytes: u64) -> Result<Self, InstantError> {
        if max_spool_bytes == 0 {
            return Err(InstantError::InvalidSpoolQuota);
        }
        Ok(Self {
            state: InstantState::Idle,
            journal_revision: 0,
            max_spool_bytes,
            retained_spool_bytes: 0,
            segments: BTreeMap::new(),
            recover_to: None,
            published_identity: None,
        })
    }

    #[must_use]
    pub const fn state(&self) -> InstantState {
        self.state
    }

    #[must_use]
    pub const fn journal_revision(&self) -> u64 {
        self.journal_revision
    }

    #[must_use]
    pub const fn retained_spool_bytes(&self) -> u64 {
        self.retained_spool_bytes
    }

    #[must_use]
    pub fn segments(&self) -> impl ExactSizeIterator<Item = &SegmentRecord> {
        self.segments.values()
    }

    pub fn begin(&mut self) -> Result<(), InstantError> {
        self.require_state(&[InstantState::Idle])?;
        self.state = InstantState::Recording;
        self.bump_revision()
    }

    pub fn set_network_available(&mut self, available: bool) -> Result<bool, InstantError> {
        let next = match (self.state, available) {
            (InstantState::Recording, false) => InstantState::Offline,
            (InstantState::Offline, true) => InstantState::Recording,
            (InstantState::Recording, true) | (InstantState::Offline, false) => return Ok(false),
            _ => return Err(InstantError::InvalidState(self.state)),
        };
        self.state = next;
        self.bump_revision()?;
        Ok(true)
    }

    /// Adds a durable local segment. Replaying an identical segment is a no-op;
    /// reusing an index for different bytes is a conflict.
    pub fn spool_segment(&mut self, descriptor: SegmentDescriptor) -> Result<bool, InstantError> {
        self.require_state(&[InstantState::Recording, InstantState::Offline])?;
        let descriptor = descriptor.validate()?;
        if let Some(existing) = self.segments.get(&descriptor.index) {
            return if existing.descriptor == descriptor {
                Ok(false)
            } else {
                Err(InstantError::SegmentConflict(descriptor.index))
            };
        }

        let next_bytes = self
            .retained_spool_bytes
            .checked_add(descriptor.bytes)
            .ok_or(InstantError::SpoolQuotaExceeded)?;
        if next_bytes > self.max_spool_bytes {
            return Err(InstantError::SpoolQuotaExceeded);
        }
        self.retained_spool_bytes = next_bytes;
        self.segments.insert(
            descriptor.index,
            SegmentRecord {
                descriptor,
                state: SegmentState::Spooled,
                spool_retained: true,
            },
        );
        self.bump_revision()?;
        Ok(true)
    }

    pub fn mark_uploaded(&mut self, index: u32) -> Result<bool, InstantError> {
        self.require_state(&[
            InstantState::Recording,
            InstantState::Offline,
            InstantState::Recovering,
        ])?;
        let record = self
            .segments
            .get_mut(&index)
            .ok_or(InstantError::UnknownSegment(index))?;
        match record.state {
            SegmentState::Spooled => {
                record.state = SegmentState::Uploaded;
                self.bump_revision()?;
                Ok(true)
            }
            SegmentState::Uploaded | SegmentState::Verified => Ok(false),
        }
    }

    pub fn mark_verified(&mut self, index: u32) -> Result<bool, InstantError> {
        self.require_state(&[
            InstantState::Recording,
            InstantState::Offline,
            InstantState::Recovering,
        ])?;
        let record = self
            .segments
            .get_mut(&index)
            .ok_or(InstantError::UnknownSegment(index))?;
        match record.state {
            SegmentState::Uploaded => {
                record.state = SegmentState::Verified;
                self.bump_revision()?;
                Ok(true)
            }
            SegmentState::Verified => Ok(false),
            SegmentState::Spooled => Err(InstantError::SegmentNotUploaded(index)),
        }
    }

    /// Releases verified local bytes while retaining the journal metadata needed
    /// to resume/finalize without reuploading the part.
    pub fn release_verified_spool(&mut self, index: u32) -> Result<bool, InstantError> {
        self.require_state(&[
            InstantState::Recording,
            InstantState::Offline,
            InstantState::Recovering,
            InstantState::Finalizing,
        ])?;
        let record = self
            .segments
            .get_mut(&index)
            .ok_or(InstantError::UnknownSegment(index))?;
        if record.state != SegmentState::Verified {
            return Err(InstantError::SegmentNotVerified(index));
        }
        if !record.spool_retained {
            return Ok(false);
        }
        record.spool_retained = false;
        self.retained_spool_bytes = self
            .retained_spool_bytes
            .checked_sub(record.descriptor.bytes)
            .ok_or(InstantError::JournalCorrupt)?;
        self.bump_revision()?;
        Ok(true)
    }

    pub fn process_crashed(&mut self) -> Result<bool, InstantError> {
        if self.state == InstantState::Recovering {
            return Ok(false);
        }
        self.require_state(&[
            InstantState::Recording,
            InstantState::Offline,
            InstantState::Finalizing,
        ])?;
        self.recover_to = Some(self.state);
        self.state = InstantState::Recovering;
        self.bump_revision()?;
        Ok(true)
    }

    pub fn resume_after_crash(&mut self) -> Result<InstantState, InstantError> {
        self.require_state(&[InstantState::Recovering])?;
        let next = self.recover_to.take().ok_or(InstantError::JournalCorrupt)?;
        self.state = next;
        self.bump_revision()?;
        Ok(next)
    }

    pub fn request_finalize(&mut self) -> Result<FinalizeManifest, InstantError> {
        if self.state == InstantState::Finalizing {
            return self.build_manifest();
        }
        self.require_state(&[InstantState::Recording, InstantState::Offline])?;
        let manifest = self.build_manifest()?;
        self.state = InstantState::Finalizing;
        self.bump_revision()?;
        Ok(FinalizeManifest {
            journal_revision: self.journal_revision,
            ..manifest
        })
    }

    /// Publishes a finalized manifest exactly once. Replaying the same identity
    /// is stable; a second identity cannot replace the published recording.
    pub fn publish(&mut self, immutable_identity: impl Into<String>) -> Result<bool, InstantError> {
        let immutable_identity = immutable_identity.into();
        validate_identity(&immutable_identity)?;
        if self.state == InstantState::Ready {
            return if self.published_identity.as_deref() == Some(&immutable_identity) {
                Ok(false)
            } else {
                Err(InstantError::PublishConflict)
            };
        }
        self.require_state(&[InstantState::Finalizing])?;
        self.published_identity = Some(immutable_identity);
        self.state = InstantState::Ready;
        self.retained_spool_bytes = 0;
        for segment in self.segments.values_mut() {
            segment.spool_retained = false;
        }
        self.bump_revision()?;
        Ok(true)
    }

    pub fn cancel(&mut self) -> Result<bool, InstantError> {
        if self.state == InstantState::Cancelled {
            return Ok(false);
        }
        if self.state.is_terminal() {
            return Err(InstantError::InvalidState(self.state));
        }
        self.state = InstantState::Cancelled;
        self.retained_spool_bytes = 0;
        for segment in self.segments.values_mut() {
            segment.spool_retained = false;
        }
        self.bump_revision()?;
        Ok(true)
    }

    pub fn fail(&mut self, recoverable: bool) -> Result<(), InstantError> {
        if self.state.is_terminal() || self.state == InstantState::Recovering {
            return Err(InstantError::InvalidState(self.state));
        }
        if recoverable {
            self.recover_to = Some(self.state);
            self.state = InstantState::Recovering;
        } else {
            self.state = InstantState::Failed;
        }
        self.bump_revision()
    }

    fn build_manifest(&self) -> Result<FinalizeManifest, InstantError> {
        if self.segments.is_empty() {
            return Err(InstantError::NoSegments);
        }
        let mut expected_index = 0_u32;
        let mut expected_start = 0_u64;
        let mut total_bytes = 0_u64;
        let mut checksums = Vec::with_capacity(self.segments.len());
        for (index, segment) in &self.segments {
            if *index != expected_index || segment.descriptor.start_ns != expected_start {
                return Err(InstantError::SegmentGap {
                    expected_index,
                    found_index: *index,
                });
            }
            if segment.state != SegmentState::Verified {
                return Err(InstantError::SegmentNotVerified(*index));
            }
            expected_index = expected_index
                .checked_add(1)
                .ok_or(InstantError::JournalCorrupt)?;
            expected_start = expected_start
                .checked_add(segment.descriptor.duration_ns)
                .ok_or(InstantError::JournalCorrupt)?;
            total_bytes = total_bytes
                .checked_add(segment.descriptor.bytes)
                .ok_or(InstantError::JournalCorrupt)?;
            checksums.push(segment.descriptor.checksum_sha256.clone());
        }
        Ok(FinalizeManifest {
            protocol_version: INSTANT_PROTOCOL_VERSION,
            journal_revision: self.journal_revision,
            segment_count: expected_index,
            total_bytes,
            duration_ns: expected_start,
            checksums,
        })
    }

    fn require_state(&self, allowed: &[InstantState]) -> Result<(), InstantError> {
        if allowed.contains(&self.state) {
            Ok(())
        } else {
            Err(InstantError::InvalidState(self.state))
        }
    }

    fn bump_revision(&mut self) -> Result<(), InstantError> {
        self.journal_revision = self
            .journal_revision
            .checked_add(1)
            .ok_or(InstantError::JournalCorrupt)?;
        Ok(())
    }
}

fn validate_identity(identity: &str) -> Result<(), InstantError> {
    if identity.is_empty()
        || identity.len() > 256
        || identity.starts_with('/')
        || identity
            .split('/')
            .any(|segment| segment.is_empty() || segment == "..")
    {
        return Err(InstantError::InvalidPublishIdentity);
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InstantError {
    #[error("local spool quota must be non-zero")]
    InvalidSpoolQuota,
    #[error("Instant session cannot perform this operation while {0:?}")]
    InvalidState(InstantState),
    #[error("segment metadata is invalid")]
    InvalidSegment,
    #[error("segment checksum must be a SHA-256 hex digest")]
    InvalidChecksum,
    #[error("local spool quota exceeded")]
    SpoolQuotaExceeded,
    #[error("segment {0} conflicts with the journal")]
    SegmentConflict(u32),
    #[error("segment {0} is not in the journal")]
    UnknownSegment(u32),
    #[error("segment {0} has not been uploaded")]
    SegmentNotUploaded(u32),
    #[error("segment {0} has not been verified")]
    SegmentNotVerified(u32),
    #[error("segment sequence gap: expected {expected_index}, found {found_index}")]
    SegmentGap {
        expected_index: u32,
        found_index: u32,
    },
    #[error("cannot finalize an Instant session without segments")]
    NoSegments,
    #[error("Instant journal is corrupt or overflowed")]
    JournalCorrupt,
    #[error("immutable publish identity is invalid")]
    InvalidPublishIdentity,
    #[error("a different immutable recording is already published")]
    PublishConflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(index: u32, start_ns: u64) -> SegmentDescriptor {
        SegmentDescriptor {
            index,
            start_ns,
            duration_ns: 1_000,
            bytes: 100,
            checksum_sha256: format!("{index:064x}"),
        }
    }

    #[test]
    fn offline_segments_resume_without_duplicate_upload() {
        let mut session = InstantSession::new(1_000).expect("session");
        session.begin().expect("begin");
        session.set_network_available(false).expect("offline");
        assert!(session.spool_segment(segment(0, 0)).expect("spool"));
        assert!(!session.spool_segment(segment(0, 0)).expect("replay"));
        session.set_network_available(true).expect("online");
        assert!(session.mark_uploaded(0).expect("upload"));
        assert!(!session.mark_uploaded(0).expect("upload replay"));
        assert!(session.mark_verified(0).expect("verify"));
        assert!(!session.mark_verified(0).expect("verify replay"));
        let manifest = session.request_finalize().expect("finalize");
        assert_eq!(manifest.segment_count, 1);
        assert!(
            session
                .publish("videos/v1/source/revision-1")
                .expect("publish")
        );
        assert!(!format!("{session:?}").contains("videos/v1"));
        assert!(
            !session
                .publish("videos/v1/source/revision-1")
                .expect("replay")
        );
    }

    #[test]
    fn crash_restores_the_exact_prior_state() {
        let mut session = InstantSession::new(1_000).expect("session");
        session.begin().expect("begin");
        session.set_network_available(false).expect("offline");
        session.process_crashed().expect("crash");
        assert_eq!(session.state(), InstantState::Recovering);
        assert_eq!(
            session.resume_after_crash().expect("resume"),
            InstantState::Offline
        );
    }

    #[test]
    fn out_of_order_segments_cannot_publish() {
        let mut session = InstantSession::new(1_000).expect("session");
        session.begin().expect("begin");
        session.spool_segment(segment(1, 1_000)).expect("spool");
        session.mark_uploaded(1).expect("uploaded");
        session.mark_verified(1).expect("verified");
        assert!(matches!(
            session.request_finalize(),
            Err(InstantError::SegmentGap { .. })
        ));
    }

    #[test]
    fn cancel_cannot_resurrect_a_recording() {
        let mut session = InstantSession::new(1_000).expect("session");
        session.begin().expect("begin");
        session.spool_segment(segment(0, 0)).expect("spool");
        session.cancel().expect("cancel");
        assert_eq!(session.retained_spool_bytes(), 0);
        assert!(matches!(
            session.mark_uploaded(0),
            Err(InstantError::InvalidState(InstantState::Cancelled))
        ));
    }
}
