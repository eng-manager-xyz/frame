//! Bounded, privacy-safe CSP report normalization.

use serde_json::Value;

const MAX_REPORTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspDirective {
    Default,
    Script,
    Style,
    Image,
    Media,
    Connect,
    Font,
    Worker,
    Manifest,
    FrameAncestors,
    FormAction,
    BaseUri,
    Object,
    Other,
}

impl CspDirective {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default_src",
            Self::Script => "script_src",
            Self::Style => "style_src",
            Self::Image => "img_src",
            Self::Media => "media_src",
            Self::Connect => "connect_src",
            Self::Font => "font_src",
            Self::Worker => "worker_src",
            Self::Manifest => "manifest_src",
            Self::FrameAncestors => "frame_ancestors",
            Self::FormAction => "form_action",
            Self::BaseUri => "base_uri",
            Self::Object => "object_src",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedResourceClass {
    None,
    SameOrigin,
    CrossOriginHttps,
    InsecureHttp,
    Data,
    Blob,
    BrowserExtension,
    Other,
}

impl BlockedResourceClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SameOrigin => "same_origin",
            Self::CrossOriginHttps => "cross_origin_https",
            Self::InsecureHttp => "insecure_http",
            Self::Data => "data",
            Self::Blob => "blob",
            Self::BrowserExtension => "browser_extension",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizedCspViolation {
    pub directive: CspDirective,
    pub blocked: BlockedResourceClass,
    pub report_only: bool,
}

/// Parse legacy CSP reports and Reporting API batches without retaining a URL,
/// source file, line, sample, policy text, referrer, user value, or body.
#[must_use]
pub fn sanitize_csp_reports(body: &[u8], canonical_origin: &str) -> Vec<SanitizedCspViolation> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let reports: Vec<&Value> = match &value {
        Value::Array(values) => values.iter().take(MAX_REPORTS).collect(),
        Value::Object(_) => vec![&value],
        _ => Vec::new(),
    };
    reports
        .into_iter()
        .map(|report| {
            let report = report
                .get("csp-report")
                .or_else(|| report.get("body"))
                .unwrap_or(report);
            let directive = report
                .get("effective-directive")
                .or_else(|| report.get("effectiveDirective"))
                .or_else(|| report.get("violated-directive"))
                .and_then(Value::as_str)
                .map(classify_directive)
                .unwrap_or(CspDirective::Other);
            let blocked = report
                .get("blocked-uri")
                .or_else(|| report.get("blockedURL"))
                .and_then(Value::as_str)
                .map(|value| classify_blocked(value, canonical_origin))
                .unwrap_or(BlockedResourceClass::None);
            let report_only = report
                .get("disposition")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "report");
            SanitizedCspViolation {
                directive,
                blocked,
                report_only,
            }
        })
        .collect()
}

fn classify_directive(value: &str) -> CspDirective {
    let base = value.split_ascii_whitespace().next().unwrap_or_default();
    if base.starts_with("script-src") {
        CspDirective::Script
    } else if base.starts_with("style-src") {
        CspDirective::Style
    } else {
        match base {
            "default-src" => CspDirective::Default,
            "img-src" => CspDirective::Image,
            "media-src" => CspDirective::Media,
            "connect-src" => CspDirective::Connect,
            "font-src" => CspDirective::Font,
            "worker-src" => CspDirective::Worker,
            "manifest-src" => CspDirective::Manifest,
            "frame-ancestors" => CspDirective::FrameAncestors,
            "form-action" => CspDirective::FormAction,
            "base-uri" => CspDirective::BaseUri,
            "object-src" => CspDirective::Object,
            _ => CspDirective::Other,
        }
    }
}

fn classify_blocked(value: &str, canonical_origin: &str) -> BlockedResourceClass {
    if value.is_empty() {
        BlockedResourceClass::None
    } else if value == "self"
        || value == canonical_origin
        || value
            .strip_prefix(canonical_origin)
            .is_some_and(|suffix| suffix.starts_with('/'))
    {
        BlockedResourceClass::SameOrigin
    } else if value.starts_with("https://") {
        BlockedResourceClass::CrossOriginHttps
    } else if value.starts_with("http://") {
        BlockedResourceClass::InsecureHttp
    } else if value.starts_with("data:") {
        BlockedResourceClass::Data
    } else if value.starts_with("blob:") {
        BlockedResourceClass::Blob
    } else if ["chrome-extension:", "moz-extension:", "safari-extension:"]
        .iter()
        .any(|scheme| value.starts_with(scheme))
    {
        BlockedResourceClass::BrowserExtension
    } else {
        BlockedResourceClass::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_report_is_reduced_to_bounded_enums() {
        let report = br#"{
          "csp-report": {
            "document-uri": "https://frame.engmanager.xyz/dashboard?token=secret",
            "referrer": "https://attacker.example/private",
            "violated-directive": "script-src-elem 'self'",
            "blocked-uri": "https://attacker.example/payload.js?secret=value",
            "source-file": "https://frame.engmanager.xyz/private-user-path.js",
            "script-sample": "bearer secret",
            "disposition": "enforce"
          }
        }"#;
        assert_eq!(
            sanitize_csp_reports(report, "https://frame.engmanager.xyz"),
            vec![SanitizedCspViolation {
                directive: CspDirective::Script,
                blocked: BlockedResourceClass::CrossOriginHttps,
                report_only: false,
            }]
        );
        let debug = format!(
            "{:?}",
            sanitize_csp_reports(report, "https://frame.engmanager.xyz")
        );
        for secret in ["secret", "attacker.example", "private-user-path"] {
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn reporting_api_batches_are_capped_and_malformed_input_is_ignored() {
        let reports = (0..16)
            .map(|_| {
                serde_json::json!({
                    "type": "csp-violation",
                    "body": {
                        "effectiveDirective": "connect-src",
                        "blockedURL": "http://insecure.example",
                        "disposition": "report"
                    }
                })
            })
            .collect::<Vec<_>>();
        let reports = serde_json::to_vec(&reports).expect("report fixture");
        let sanitized = sanitize_csp_reports(&reports, "https://frame.engmanager.xyz");
        assert_eq!(sanitized.len(), MAX_REPORTS);
        assert!(sanitized.iter().all(|report| {
            report.directive == CspDirective::Connect
                && report.blocked == BlockedResourceClass::InsecureHttp
                && report.report_only
        }));
        assert!(sanitize_csp_reports(b"not-json", "https://frame.engmanager.xyz").is_empty());
        assert_eq!(
            classify_blocked(
                "https://frame.engmanager.xyz.evil.test/script.js",
                "https://frame.engmanager.xyz"
            ),
            BlockedResourceClass::CrossOriginHttps
        );
    }
}
