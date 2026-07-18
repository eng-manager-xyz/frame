//! Exact D1 read authority for Cap's desktop organization custom-domain route.
//!
//! The caller supplies only the authenticated actor identity. The active
//! organization and domain are always selected inside D1; a caller-provided
//! organization or domain can never influence this read.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const READ_FOR_ACTOR_SQL: &str =
    include_str!("../queries/legacy_org_custom_domain/read_for_actor.sql");
const MAX_ACTOR_ID_BYTES: usize = 256;
// Cap stores at most 255 utf8mb4 characters. Keep a defensive byte bound that
// still admits every value the pinned source column can contain.
const MAX_LEGACY_CUSTOM_DOMAIN_BYTES: usize = 255 * 4;
const LEGACY_DESKTOP_ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:3001",
    "http://localhost:3000",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

pub(crate) const LEGACY_ORG_CUSTOM_DOMAIN_SUCCESS_CONTENT_TYPE: &str = "application/json";
pub(crate) const LEGACY_ORG_CUSTOM_DOMAIN_UNAUTHENTICATED_CONTENT_TYPE: &str =
    "text/plain; charset=UTF-8";
pub(crate) const LEGACY_ORG_CUSTOM_DOMAIN_UNAUTHENTICATED_BODY: &str = "User not authenticated";
pub(crate) const LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_CONTENT_TYPE: &str = "application/json";
pub(crate) const LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_BODY: &str =
    r#"{"error":"Failed to fetch custom domain"}"#;
pub(crate) const LEGACY_ORG_CUSTOM_DOMAIN_VERIFIED_RUNTIME_TYPE: &str =
    "iso_timestamp_string_or_null";

pub(crate) fn legacy_desktop_cors_headers(
    request_origin: Option<&str>,
    configured_origin: &str,
) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(3);
    if let Some(origin) = request_origin.filter(|origin| {
        *origin == configured_origin || LEGACY_DESKTOP_ALLOWED_ORIGINS.contains(origin)
    }) {
        headers.push(("Access-Control-Allow-Origin".into(), origin.into()));
    }
    headers.push(("Access-Control-Allow-Credentials".into(), "true".into()));
    headers.push(("Vary".into(), "Origin".into()));
    headers
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LegacyOrganizationCustomDomainV1 {
    custom_domain: Option<String>,
    domain_verified: Option<String>,
}

impl LegacyOrganizationCustomDomainV1 {
    const fn none() -> Self {
        Self {
            custom_domain: None,
            domain_verified: None,
        }
    }

    #[must_use]
    pub(crate) fn custom_domain(&self) -> Option<&str> {
        self.custom_domain.as_deref()
    }

    #[must_use]
    pub(crate) fn domain_verified(&self) -> Option<&str> {
        self.domain_verified.as_deref()
    }

    pub(crate) fn exact_json_body(&self) -> Result<Vec<u8>, LegacyOrganizationCustomDomainErrorV1> {
        serde_json::to_vec(self).map_err(|_| LegacyOrganizationCustomDomainErrorV1::Corrupt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyOrganizationCustomDomainErrorV1 {
    InvalidActor,
    Unavailable,
    Corrupt,
}

#[async_trait(?Send)]
pub(crate) trait LegacyOrganizationCustomDomainAuthorityV1 {
    async fn read_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacyOrganizationCustomDomainV1, LegacyOrganizationCustomDomainErrorV1>;
}

pub(crate) struct D1LegacyOrganizationCustomDomainAuthorityV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyOrganizationCustomDomainAuthorityV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }
}

#[derive(Debug, Deserialize)]
struct CustomDomainRow {
    organization_present: i64,
    projection_present: i64,
    custom_domain: Option<String>,
    domain_verified: Option<String>,
}

#[async_trait(?Send)]
impl LegacyOrganizationCustomDomainAuthorityV1 for D1LegacyOrganizationCustomDomainAuthorityV1<'_> {
    async fn read_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacyOrganizationCustomDomainV1, LegacyOrganizationCustomDomainErrorV1> {
        if !valid_actor_id(actor_id) {
            return Err(LegacyOrganizationCustomDomainErrorV1::InvalidActor);
        }
        let result = self
            .database
            .prepare(READ_FOR_ACTOR_SQL)
            .bind(&[JsValue::from_str(actor_id)])
            .map_err(|_| LegacyOrganizationCustomDomainErrorV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyOrganizationCustomDomainErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyOrganizationCustomDomainErrorV1::Unavailable);
        }
        let rows = result
            .results::<serde_json::Value>()
            .map_err(|_| LegacyOrganizationCustomDomainErrorV1::Unavailable)?
            .into_iter()
            .map(|row| {
                serde_json::from_value::<CustomDomainRow>(row)
                    .map_err(|_| LegacyOrganizationCustomDomainErrorV1::Corrupt)
            })
            .collect::<Result<Vec<_>, _>>()?;
        decode_rows(rows)
    }
}

fn valid_actor_id(actor_id: &str) -> bool {
    !actor_id.is_empty()
        && actor_id.len() <= MAX_ACTOR_ID_BYTES
        && actor_id.is_ascii()
        && !actor_id.bytes().any(|byte| byte.is_ascii_control())
}

fn decode_rows(
    mut rows: Vec<CustomDomainRow>,
) -> Result<LegacyOrganizationCustomDomainV1, LegacyOrganizationCustomDomainErrorV1> {
    if rows.len() > 1 {
        return Err(LegacyOrganizationCustomDomainErrorV1::Corrupt);
    }
    let Some(row) = rows.pop() else {
        // This matches the pinned left-join handler's `result?.field ?? null`
        // behavior if an authenticated user disappears between auth and read.
        return Ok(LegacyOrganizationCustomDomainV1::none());
    };
    match (row.organization_present, row.projection_present) {
        (0, 0) => {
            if row.custom_domain.is_none() && row.domain_verified.is_none() {
                return Ok(LegacyOrganizationCustomDomainV1::none());
            }
            return Err(LegacyOrganizationCustomDomainErrorV1::Corrupt);
        }
        (1, 1) => {}
        // An organization without its source projection is an incomplete
        // import, not proof that both legacy source fields were null.
        _ => return Err(LegacyOrganizationCustomDomainErrorV1::Corrupt),
    }
    let custom_domain = row.custom_domain.map(normalize_custom_domain).transpose()?;
    if row
        .domain_verified
        .as_deref()
        .is_some_and(|value| !valid_iso_timestamp(value))
    {
        return Err(LegacyOrganizationCustomDomainErrorV1::Corrupt);
    }
    Ok(LegacyOrganizationCustomDomainV1 {
        custom_domain,
        domain_verified: row.domain_verified,
    })
}

fn valid_iso_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
        || bytes.iter().enumerate().any(|(index, byte)| {
            !matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) && !byte.is_ascii_digit()
        })
    {
        return false;
    }
    let number = |start: usize, end: usize| {
        value
            .get(start..end)
            .and_then(|part| part.parse::<u16>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day) && hour <= 23 && minute <= 59 && second <= 59
}

fn normalize_custom_domain(
    custom_domain: String,
) -> Result<String, LegacyOrganizationCustomDomainErrorV1> {
    if custom_domain.len() > MAX_LEGACY_CUSTOM_DOMAIN_BYTES {
        return Err(LegacyOrganizationCustomDomainErrorV1::Corrupt);
    }
    if custom_domain.is_empty()
        || custom_domain.starts_with("http://")
        || custom_domain.starts_with("https://")
    {
        Ok(custom_domain)
    } else {
        Ok(format!("https://{custom_domain}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_with_presence(
        organization_present: i64,
        projection_present: i64,
        custom_domain: Option<&str>,
        domain_verified: Option<&str>,
    ) -> CustomDomainRow {
        CustomDomainRow {
            organization_present,
            projection_present,
            custom_domain: custom_domain.map(str::to_owned),
            domain_verified: domain_verified.map(str::to_owned),
        }
    }

    fn row(custom_domain: Option<&str>, domain_verified: Option<&str>) -> CustomDomainRow {
        row_with_presence(1, 1, custom_domain, domain_verified)
    }

    #[test]
    fn bounded_query_derives_tenant_only_from_the_authenticated_actor() {
        assert_eq!(READ_FOR_ACTOR_SQL.matches('?').count(), 1);
        for token in [
            "u.active_organization_id",
            "LEFT JOIN organizations o",
            "CASE WHEN o.id IS NULL THEN 0 ELSE 1 END AS organization_present",
            "legacy_org_custom_domain_projection_v1",
            "p.organization_id = o.id",
            "WHERE u.id = ?1",
            "LIMIT 1",
        ] {
            assert!(
                READ_FOR_ACTOR_SQL.contains(token),
                "missing SQL guard: {token}"
            );
        }
        assert!(!READ_FOR_ACTOR_SQL.contains("?2"));
        for forbidden in [
            "u.status",
            "u.deleted_at_ms",
            "organization_members",
            "storage_custom_domains_v1",
        ] {
            assert!(
                !READ_FOR_ACTOR_SQL.contains(forbidden),
                "source handler does not impose filter: {forbidden}"
            );
        }
    }

    #[test]
    fn exact_url_prefix_and_nullable_semantics_match_the_pinned_handler() {
        const VERIFIED: &str = "2026-07-16T12:34:56.789Z";
        for (input, verified, expected_domain, expected_verified) in [
            (None, None, None, None),
            (Some(""), None, Some(""), None),
            (
                Some("recordings.example.com"),
                None,
                Some("https://recordings.example.com"),
                None,
            ),
            (
                Some("http://recordings.example.com"),
                Some(VERIFIED),
                Some("http://recordings.example.com"),
                Some(VERIFIED),
            ),
            (
                Some("https://recordings.example.com"),
                Some(VERIFIED),
                Some("https://recordings.example.com"),
                Some(VERIFIED),
            ),
            (
                Some("HTTPS://recordings.example.com"),
                Some(VERIFIED),
                Some("https://HTTPS://recordings.example.com"),
                Some(VERIFIED),
            ),
            (None, Some(VERIFIED), None, Some(VERIFIED)),
            (
                Some("münchen.example"),
                None,
                Some("https://münchen.example"),
                None,
            ),
            (
                Some("line\nbreak.example"),
                None,
                Some("https://line\nbreak.example"),
                None,
            ),
        ] {
            let decoded = decode_rows(vec![row(input, verified)]).expect("exact row");
            assert_eq!(decoded.custom_domain(), expected_domain);
            assert_eq!(decoded.domain_verified(), expected_verified);
        }
        assert_eq!(
            decode_rows(Vec::new()).expect("missing user follows optional row semantics"),
            LegacyOrganizationCustomDomainV1::none()
        );
        assert_eq!(
            decode_rows(vec![row_with_presence(0, 0, None, None)])
                .expect("missing active organization follows left-join semantics"),
            LegacyOrganizationCustomDomainV1::none()
        );
    }

    #[test]
    fn exact_success_and_failure_bodies_and_media_types_are_frozen() {
        const VERIFIED: &str = "2026-07-16T12:34:56.789Z";
        let none = decode_rows(vec![row(None, None)]).expect("nullable response");
        assert_eq!(
            none.exact_json_body().expect("JSON"),
            br#"{"custom_domain":null,"domain_verified":null}"#
        );
        let active = decode_rows(vec![row(Some("recordings.example.com"), Some(VERIFIED))])
            .expect("verified domain response");
        assert_eq!(
            active.exact_json_body().expect("JSON"),
            br#"{"custom_domain":"https://recordings.example.com","domain_verified":"2026-07-16T12:34:56.789Z"}"#
        );
        assert_eq!(
            LEGACY_ORG_CUSTOM_DOMAIN_VERIFIED_RUNTIME_TYPE,
            "iso_timestamp_string_or_null"
        );
        assert_eq!(
            LEGACY_ORG_CUSTOM_DOMAIN_SUCCESS_CONTENT_TYPE,
            "application/json"
        );
        assert_eq!(
            LEGACY_ORG_CUSTOM_DOMAIN_UNAUTHENTICATED_CONTENT_TYPE,
            "text/plain; charset=UTF-8"
        );
        assert_eq!(
            LEGACY_ORG_CUSTOM_DOMAIN_UNAUTHENTICATED_BODY,
            "User not authenticated"
        );
        assert_eq!(
            LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_CONTENT_TYPE,
            "application/json"
        );
        assert_eq!(
            LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_BODY,
            r#"{"error":"Failed to fetch custom domain"}"#
        );
    }

    #[test]
    fn desktop_mount_cors_headers_match_the_pinned_get_middleware() {
        assert_eq!(
            legacy_desktop_cors_headers(
                Some("https://frame.engmanager.xyz"),
                "https://frame.engmanager.xyz"
            ),
            [
                (
                    "Access-Control-Allow-Origin".into(),
                    "https://frame.engmanager.xyz".into()
                ),
                ("Access-Control-Allow-Credentials".into(), "true".into()),
                ("Vary".into(), "Origin".into()),
            ]
        );
        assert_eq!(
            legacy_desktop_cors_headers(Some("tauri://localhost"), "https://frame.engmanager.xyz")
                .first(),
            Some(&(
                "Access-Control-Allow-Origin".into(),
                "tauri://localhost".into()
            ))
        );
        assert_eq!(
            legacy_desktop_cors_headers(
                Some("https://attacker.invalid"),
                "https://frame.engmanager.xyz"
            ),
            [
                ("Access-Control-Allow-Credentials".into(), "true".into()),
                ("Vary".into(), "Origin".into()),
            ]
        );
        assert_eq!(
            legacy_desktop_cors_headers(None, "https://frame.engmanager.xyz"),
            [
                ("Access-Control-Allow-Credentials".into(), "true".into()),
                ("Vary".into(), "Origin".into()),
            ]
        );
    }

    #[test]
    fn corrupt_or_ambiguous_rows_fail_closed() {
        const VERIFIED: &str = "2026-07-16T12:34:56.789Z";
        for rows in [
            vec![row_with_presence(1, 0, None, None)],
            vec![row_with_presence(0, 1, None, None)],
            vec![row_with_presence(0, 0, Some("impossible.example"), None)],
            vec![row(Some("example.com"), Some("not-a-date"))],
            vec![row(Some("example.com"), Some("2026-13-16T12:34:56.789Z"))],
            vec![row(Some("example.com"), Some("2025-02-29T12:34:56.789Z"))],
            vec![row(
                Some(&"a".repeat(MAX_LEGACY_CUSTOM_DOMAIN_BYTES + 1)),
                Some(VERIFIED),
            )],
            vec![
                row(Some("one.example.com"), Some(VERIFIED)),
                row(Some("two.example.com"), None),
            ],
        ] {
            assert_eq!(
                decode_rows(rows),
                Err(LegacyOrganizationCustomDomainErrorV1::Corrupt)
            );
        }
        for actor_id in ["", "bad\nactor"] {
            assert!(!valid_actor_id(actor_id));
        }
        assert!(!valid_actor_id(&"a".repeat(MAX_ACTOR_ID_BYTES + 1)));
        assert!(valid_actor_id("user_01HX8Z9Q7Q33C46P4T0W0M8YQF"));
    }
}
