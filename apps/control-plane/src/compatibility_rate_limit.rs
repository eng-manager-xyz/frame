//! Fail-closed D1 admission for promoted exact compatibility adapters.
//!
//! The generated parity report names a rate-limit bucket for every operation.
//! This module turns those labels into one production authority. Public routes
//! use a keyed digest of Cloudflare's canonical client address; authenticated
//! routes use a keyed principal digest. Only the active hash-key version and
//! digest are stored, never the source address or principal identifier.

use std::net::IpAddr;

use frame_application::{AuthHashKeyRing, RateLimitDecisionV1};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Error, Request, Result, send::IntoSendFuture};

use crate::browser_web_runtime;

const CLEANUP_SQL: &str =
    include_str!("../queries/api_workflow/compatibility_rate_limit_cleanup.sql");
const ADMIT_SQL: &str = include_str!("../queries/api_workflow/compatibility_rate_limit_admit.sql");
const WINDOW_MS: i64 = 60_000;
pub(crate) const RETRY_AFTER_SECONDS: u64 = 60;
const RETENTION_WINDOWS: i64 = 2;
const UNKNOWN_EDGE_SOURCE: &[u8] = b"unattributed-cloudflare-source";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompatibilityRateLimitBucketV1 {
    AuthSession,
    BillingAdmin,
    StripeWebhookIngress,
    ServiceMisc,
    ClientCompatibility,
    OrganizationLibrary,
    UploadStorage,
    CollaborationNotifications,
    DeveloperApi,
    SharePlayback,
    VideoMedia,
}

impl CompatibilityRateLimitBucketV1 {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AuthSession => "auth_session.v1",
            Self::BillingAdmin => "billing_admin.v1",
            Self::StripeWebhookIngress => "stripe_webhook_ingress.v1",
            Self::ServiceMisc => "service_misc.v1",
            Self::ClientCompatibility => "client_compatibility.v1",
            Self::OrganizationLibrary => "organization_library.v1",
            Self::UploadStorage => "upload_storage.v1",
            Self::CollaborationNotifications => "collaboration_notifications.v1",
            Self::DeveloperApi => "developer_api.v1",
            Self::SharePlayback => "share_playback.v1",
            Self::VideoMedia => "video_media.v1",
        }
    }

    const fn request_limit(self) -> i64 {
        match self {
            Self::AuthSession => 8,
            // Billing and administrator execution is both expensive and
            // independently approved; keep request-path intent staging tight.
            Self::BillingAdmin => 8,
            // Reject abusive webhook sources before a 1 MiB body/HMAC parse,
            // while leaving enough headroom for legitimate Stripe retry bursts.
            Self::StripeWebhookIngress => 120,
            // Low-cost metadata and preflight operations share this bucket.
            Self::ServiceMisc => 120,
            // The exact feed is 88,817 bytes, so bound polling more tightly.
            Self::ClientCompatibility => 12,
            Self::OrganizationLibrary => 12,
            // Cap's developer SDK and REST middleware both freeze a
            // 60-request, 60-second window. Multipart SDK calls retain the
            // report's upload-storage bucket while using that source limit.
            Self::UploadStorage => 60,
            Self::CollaborationNotifications => 30,
            Self::DeveloperApi => 60,
            Self::SharePlayback => 30,
            Self::VideoMedia => 12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompatibilityRateLimitDimensionV1 {
    Source,
    Principal,
}

impl CompatibilityRateLimitDimensionV1 {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Principal => "principal",
        }
    }
}

#[derive(Debug, Deserialize)]
struct AdmissionRow {
    request_count: i64,
}

/// Admit a public compatibility request by Cloudflare's edge-authenticated
/// client address. A missing or malformed address intentionally collapses to
/// one fail-closed shared subject instead of creating attacker-selected rows.
pub(crate) async fn admit_edge_request(
    env: &Env,
    request: &Request,
    bucket: CompatibilityRateLimitBucketV1,
    now_ms: i64,
) -> Result<RateLimitDecisionV1> {
    let source = normalized_edge_source(request.headers().get("cf-connecting-ip")?.as_deref());
    let keys = browser_web_runtime::auth_hash_keyring(env).map_err(|_| {
        Error::RustError("compatibility rate-limit authority is unavailable".into())
    })?;
    admit(
        &env.d1("DB")?,
        &keys,
        bucket,
        CompatibilityRateLimitDimensionV1::Source,
        source.as_deref().unwrap_or(UNKNOWN_EDGE_SOURCE),
        now_ms,
    )
    .await
}

/// Admit an authenticated compatibility operation by principal. This is kept
/// separate from the browser action implementation so the one-use mutation
/// grant is never consumed by a request that should receive a rate-limit
/// rejection.
pub(crate) async fn admit_principal(
    env: &Env,
    database: &D1Database,
    bucket: CompatibilityRateLimitBucketV1,
    principal_id: &str,
    now_ms: i64,
) -> Result<RateLimitDecisionV1> {
    let keys = browser_web_runtime::auth_hash_keyring(env).map_err(|_| {
        Error::RustError("compatibility rate-limit authority is unavailable".into())
    })?;
    admit(
        database,
        &keys,
        bucket,
        CompatibilityRateLimitDimensionV1::Principal,
        principal_id.as_bytes(),
        now_ms,
    )
    .await
}

async fn admit(
    database: &D1Database,
    keys: &AuthHashKeyRing,
    bucket: CompatibilityRateLimitBucketV1,
    dimension: CompatibilityRateLimitDimensionV1,
    subject: &[u8],
    now_ms: i64,
) -> Result<RateLimitDecisionV1> {
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Err(Error::RustError(
            "compatibility rate-limit clock is invalid".into(),
        ));
    }
    let digest = keys
        .compatibility_rate_limit_digest(bucket.as_str(), subject)
        .map_err(|_| Error::RustError("compatibility rate-limit subject is invalid".into()))?;
    let cleanup = database
        .prepare(CLEANUP_SQL)
        .bind(&[JsValue::from_f64(now_ms as f64)])?
        .run()
        .into_send()
        .await?;
    if !cleanup.success() {
        return Err(Error::RustError(
            "compatibility rate-limit cleanup failed".into(),
        ));
    }
    let window_reset_before = now_ms.saturating_sub(WINDOW_MS);
    let gc_at_ms = now_ms
        .checked_add(WINDOW_MS * RETENTION_WINDOWS)
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or_else(|| Error::RustError("compatibility rate-limit clock is invalid".into()))?;
    let admitted = database
        .prepare(ADMIT_SQL)
        .bind(&[
            JsValue::from_str(bucket.as_str()),
            JsValue::from_str(dimension.as_str()),
            JsValue::from_f64(f64::from(digest.key_version.get())),
            JsValue::from_str(digest.digest.expose_for_verification()),
            JsValue::from_f64(now_ms as f64),
            JsValue::from_f64(gc_at_ms as f64),
            JsValue::from_f64(window_reset_before as f64),
            JsValue::from_f64(bucket.request_limit() as f64),
        ])?
        .first::<AdmissionRow>(None)
        .into_send()
        .await?;
    match admitted {
        Some(row) if (1..=bucket.request_limit()).contains(&row.request_count) => {
            Ok(RateLimitDecisionV1::Allowed)
        }
        Some(_) => Err(Error::RustError(
            "compatibility rate-limit postcondition failed".into(),
        )),
        None => Ok(RateLimitDecisionV1::Rejected {
            retry_after_ms: RETRY_AFTER_SECONDS * 1_000,
        }),
    }
}

fn normalized_edge_source(value: Option<&str>) -> Option<Vec<u8>> {
    let value = value?;
    if value.trim() != value || value.len() > 64 {
        return None;
    }
    value
        .parse::<IpAddr>()
        .ok()
        .map(|address| address.to_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_labels_and_limits_are_closed_and_bounded() {
        assert_eq!(
            CompatibilityRateLimitBucketV1::BillingAdmin.as_str(),
            "billing_admin.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::BillingAdmin.request_limit(),
            8
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::StripeWebhookIngress.as_str(),
            "stripe_webhook_ingress.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::StripeWebhookIngress.request_limit(),
            120
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::ServiceMisc.as_str(),
            "service_misc.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::ServiceMisc.request_limit(),
            120
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::ClientCompatibility.as_str(),
            "client_compatibility.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::ClientCompatibility.request_limit(),
            12
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::OrganizationLibrary.as_str(),
            "organization_library.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::OrganizationLibrary.request_limit(),
            12
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::CollaborationNotifications.as_str(),
            "collaboration_notifications.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::CollaborationNotifications.request_limit(),
            30
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::DeveloperApi.as_str(),
            "developer_api.v1"
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::DeveloperApi.request_limit(),
            60
        );
        assert_eq!(
            CompatibilityRateLimitBucketV1::UploadStorage.request_limit(),
            60
        );
    }

    #[test]
    fn edge_source_is_canonical_or_collapses_to_shared_subject() {
        assert_eq!(
            normalized_edge_source(Some("192.0.2.1")),
            Some(b"192.0.2.1".to_vec())
        );
        assert_eq!(
            normalized_edge_source(Some("2001:0db8::1")),
            Some(b"2001:db8::1".to_vec())
        );
        assert_eq!(normalized_edge_source(None), None);
        assert_eq!(normalized_edge_source(Some(" 192.0.2.1")), None);
        assert_eq!(normalized_edge_source(Some("attacker-selected")), None);
    }

    #[test]
    fn admission_query_is_atomic_and_never_increments_past_the_limit() {
        assert!(ADMIT_SQL.contains("ON CONFLICT(bucket, dimension, key_version, subject_digest)"));
        assert!(ADMIT_SQL.contains("request_count < ?8"));
        assert!(ADMIT_SQL.contains("RETURNING request_count"));
        assert!(CLEANUP_SQL.contains("LIMIT 16"));
    }
}
