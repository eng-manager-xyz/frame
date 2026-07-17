//! First-party authenticated wire contracts for Frame applications.
//!
//! This crate is deliberately separate from the anonymous `frame-client`
//! boundary. It contains immutable request and response identities only: no
//! bearer credentials, provider handles, storage keys, or runtime types.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const INSTANT_FINALIZE_SCHEMA_VERSION: u16 = 1;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantFinalizeContractError;

impl fmt::Display for InstantFinalizeContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the authenticated Instant finalize contract is invalid")
    }
}

impl std::error::Error for InstantFinalizeContractError {}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstantFinalizeRequestV1 {
    pub schema_version: u16,
    pub tenant_id: String,
    pub session_id: String,
    pub operation_id: String,
    pub upload_id: String,
    pub video_id: String,
    pub ordered_parts_sha256: String,
    pub object_version: String,
    pub job_id: String,
    pub job_generation: u64,
    pub request_sha256: String,
}

impl fmt::Debug for InstantFinalizeRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeRequestV1")
            .field("schema_version", &self.schema_version)
            .field("job_generation", &self.job_generation)
            .finish_non_exhaustive()
    }
}

impl InstantFinalizeRequestV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: String,
        session_id: String,
        operation_id: String,
        upload_id: String,
        video_id: String,
        ordered_parts_sha256: String,
        object_version: String,
        job_id: String,
        job_generation: u64,
    ) -> Result<Self, InstantFinalizeContractError> {
        let mut request = Self {
            schema_version: INSTANT_FINALIZE_SCHEMA_VERSION,
            tenant_id,
            session_id,
            operation_id,
            upload_id,
            video_id,
            ordered_parts_sha256,
            object_version,
            job_id,
            job_generation,
            request_sha256: String::new(),
        };
        request.request_sha256 = request.compute_request_sha256();
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), InstantFinalizeContractError> {
        let uuids = [
            &self.tenant_id,
            &self.session_id,
            &self.operation_id,
            &self.upload_id,
            &self.video_id,
            &self.job_id,
        ];
        if self.schema_version != INSTANT_FINALIZE_SCHEMA_VERSION
            || uuids.iter().any(|value| !valid_uuid(value))
            || !(1..=MAX_SAFE_INTEGER).contains(&self.job_generation)
            || !valid_sha256(&self.ordered_parts_sha256)
            || !valid_sha256(&self.object_version)
            || !valid_sha256(&self.request_sha256)
            || self.compute_request_sha256() != self.request_sha256
        {
            return Err(InstantFinalizeContractError);
        }
        Ok(())
    }

    #[must_use]
    pub fn compute_request_sha256(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame.instant.finalize-wire.v1\0");
        append_u16(&mut digest, self.schema_version);
        for value in [
            &self.tenant_id,
            &self.session_id,
            &self.upload_id,
            &self.video_id,
        ] {
            append_text(&mut digest, value);
        }
        for value in [
            &self.ordered_parts_sha256,
            &self.object_version,
            &self.job_id,
        ] {
            append_text(&mut digest, value);
        }
        append_u64(&mut digest, self.job_generation);
        hex(&digest.finalize())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstantFinalizeStateV1 {
    Pending,
    Published,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstantFinalizeReceiptV1 {
    pub schema_version: u16,
    pub state: InstantFinalizeStateV1,
    pub request_sha256: String,
    pub publication_id: Option<String>,
    pub job_id: String,
    pub job_generation: u64,
    pub upload_id: String,
    pub object_version: String,
    pub distribution_eligible: bool,
}

impl fmt::Debug for InstantFinalizeReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantFinalizeReceiptV1")
            .field("schema_version", &self.schema_version)
            .field("state", &self.state)
            .field("job_generation", &self.job_generation)
            .field("distribution_eligible", &self.distribution_eligible)
            .finish_non_exhaustive()
    }
}

impl InstantFinalizeReceiptV1 {
    pub fn validate_for(
        &self,
        request: &InstantFinalizeRequestV1,
    ) -> Result<(), InstantFinalizeContractError> {
        request.validate()?;
        let published = self.state == InstantFinalizeStateV1::Published;
        if self.schema_version != INSTANT_FINALIZE_SCHEMA_VERSION
            || self.request_sha256 != request.request_sha256
            || self.job_id != request.job_id
            || self.job_generation != request.job_generation
            || self.upload_id != request.upload_id
            || self.object_version != request.object_version
            || published != self.publication_id.as_deref().is_some_and(valid_uuid)
            || published != self.distribution_eligible
        {
            return Err(InstantFinalizeContractError);
        }
        Ok(())
    }
}

fn append_text(digest: &mut Sha256, value: &str) {
    let length = u32::try_from(value.len()).unwrap_or(u32::MAX);
    digest.update(length.to_be_bytes());
    digest.update(value.as_bytes());
}

fn append_u16(digest: &mut Sha256, value: u16) {
    digest.update(value.to_be_bytes());
}

fn append_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_be_bytes());
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .is_ok_and(|uuid| !uuid.is_nil() && uuid.as_hyphenated().to_string() == value)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
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

    fn request() -> InstantFinalizeRequestV1 {
        InstantFinalizeRequestV1::new(
            "018f47a6-7b1c-7f55-8f39-8f8a86900001".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900002".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900003".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900004".into(),
            "018f47a6-7b1c-7f55-8f39-8f8a86900005".into(),
            "b".repeat(64),
            "c".repeat(64),
            "018f47a6-7b1c-7f55-8f39-8f8a86900006".into(),
            1,
        )
        .expect("request")
    }

    fn published_receipt(request: &InstantFinalizeRequestV1) -> InstantFinalizeReceiptV1 {
        InstantFinalizeReceiptV1 {
            schema_version: INSTANT_FINALIZE_SCHEMA_VERSION,
            state: InstantFinalizeStateV1::Published,
            request_sha256: request.request_sha256.clone(),
            publication_id: Some("018f47a6-7b1c-7f55-8f39-8f8a86900007".into()),
            job_id: request.job_id.clone(),
            job_generation: request.job_generation,
            upload_id: request.upload_id.clone(),
            object_version: request.object_version.clone(),
            distribution_eligible: true,
        }
    }

    #[test]
    fn request_digest_is_stable_across_exact_operation_retries() {
        let request = request();
        let mut retry = request.clone();
        retry.operation_id = "018f47a6-7b1c-7f55-8f39-8f8a86900008".into();
        assert_eq!(retry.compute_request_sha256(), request.request_sha256);
        retry.validate().expect("operation retry");
        retry.object_version = "d".repeat(64);
        assert_ne!(retry.compute_request_sha256(), request.request_sha256);
        assert!(retry.validate().is_err());
    }

    #[test]
    fn receipt_binds_publication_request_job_upload_object_version_and_distribution() {
        let request = request();
        let receipt = published_receipt(&request);
        receipt.validate_for(&request).expect("valid receipt");

        let mut changed = receipt.clone();
        changed.publication_id = None;
        assert!(changed.validate_for(&request).is_err());
        let mut changed = receipt.clone();
        changed.job_generation += 1;
        assert!(changed.validate_for(&request).is_err());
        let mut changed = receipt.clone();
        changed.upload_id = "018f47a6-7b1c-7f55-8f39-8f8a86900009".into();
        assert!(changed.validate_for(&request).is_err());
        let mut changed = receipt.clone();
        changed.object_version = "d".repeat(64);
        assert!(changed.validate_for(&request).is_err());
        let mut changed = receipt;
        changed.distribution_eligible = false;
        assert!(changed.validate_for(&request).is_err());
    }

    #[test]
    fn wire_receipt_contains_only_first_party_publication_identity() {
        let request = request();
        let value = serde_json::to_value(published_receipt(&request)).expect("receipt JSON");
        let keys = value
            .as_object()
            .expect("receipt object")
            .keys()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "distribution_eligible",
                "job_generation",
                "job_id",
                "object_version",
                "publication_id",
                "request_sha256",
                "schema_version",
                "state",
                "upload_id",
            ]
        );
    }

    #[test]
    fn pending_receipt_has_no_publication_and_is_not_distributable() {
        let request = request();
        let receipt = InstantFinalizeReceiptV1 {
            schema_version: INSTANT_FINALIZE_SCHEMA_VERSION,
            state: InstantFinalizeStateV1::Pending,
            request_sha256: request.request_sha256.clone(),
            publication_id: None,
            job_id: request.job_id.clone(),
            job_generation: request.job_generation,
            upload_id: request.upload_id.clone(),
            object_version: request.object_version.clone(),
            distribution_eligible: false,
        };
        receipt.validate_for(&request).expect("pending receipt");
    }
}
