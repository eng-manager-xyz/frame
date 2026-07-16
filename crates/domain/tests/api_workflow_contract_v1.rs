use frame_domain::{
    API_CONTRACT_VERSION_V1, ApiErrorV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    CompatibilityDecisionV1, PublicMediaStatusV1,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractCases {
    schema_version: String,
    errors: Vec<ApiErrorV1>,
    media_statuses: Vec<PublicMediaStatusV1>,
    compatibility: Vec<CompatibilityCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityCase {
    policy: ClientCompatibilityPolicyV1,
    release: ClientReleaseV1,
    expected: ExpectedCompatibility,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedCompatibility {
    Current,
    Previous,
    UpgradeRequired,
    Retired,
}

impl From<ExpectedCompatibility> for CompatibilityDecisionV1 {
    fn from(value: ExpectedCompatibility) -> Self {
        match value {
            ExpectedCompatibility::Current => Self::Current,
            ExpectedCompatibility::Previous => Self::Previous,
            ExpectedCompatibility::UpgradeRequired => Self::UpgradeRequired,
            ExpectedCompatibility::Retired => Self::Retired,
        }
    }
}

fn cases() -> ContractCases {
    serde_json::from_str(include_str!(
        "../../../fixtures/api-parity/v1/contract-cases.json"
    ))
    .expect("API contract fixture")
}

#[test]
fn public_error_snapshot_has_closed_retry_semantics() {
    let cases = cases();
    assert_eq!(cases.schema_version, "frame.api-contract-cases.v1");
    assert!(!cases.errors.is_empty());
    for error in cases.errors {
        error.validate().expect("valid public error");
        assert_eq!(error.schema_version, API_CONTRACT_VERSION_V1);
        assert_eq!(error.retryable, error.code.retryable());
        assert_eq!(error.retry_after_ms.is_some(), error.code.retryable());
        let encoded = serde_json::to_string(&error).expect("encode error");
        for forbidden in [
            "provider",
            "executor",
            "object_key",
            "cookie",
            "token",
            "email",
        ] {
            assert!(!encoded.contains(forbidden), "leaked {forbidden}");
        }
    }
}

#[test]
fn provider_neutral_media_status_snapshots_validate() {
    let cases = cases();
    assert_eq!(cases.media_statuses.len(), 6);
    for status in cases.media_statuses {
        status.validate().expect("valid public media status");
        let encoded = serde_json::to_string(&status).expect("encode media status");
        for forbidden in ["cloudflare", "gstreamer", "executor", "binding", "r2"] {
            assert!(!encoded.contains(forbidden), "leaked {forbidden}");
        }
    }
}

#[test]
fn compatibility_snapshot_covers_current_previous_and_upgrade() {
    let cases = cases();
    assert_eq!(cases.compatibility.len(), 3);
    for case in cases.compatibility {
        assert_eq!(case.policy.decide(&case.release), Ok(case.expected.into()));
    }
}
