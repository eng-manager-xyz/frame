//! Checked D1 projection for Cap's anonymous `/api/video/domain-info` route.

use frame_application::{LegacyVideoDomainInfoProjectionV1, legacy_video_domain_info_select};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const VIDEO_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_video_domain_info/video_authority.sql");
const ORGANIZATION_DOMAIN_SQL: &str =
    include_str!("../queries/legacy_video_domain_info/organization_domain.sql");
const OWNER_DOMAIN_SQL: &str = include_str!("../queries/legacy_video_domain_info/owner_domain.sql");

const MAX_LEGACY_ID_BYTES: usize = 255 * 4;
const MAX_CUSTOM_DOMAIN_BYTES: usize = 255 * 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacyVideoDomainInfoReadV1 {
    Found(LegacyVideoDomainInfoProjectionV1),
    VideoNotFound,
    InvalidVideoData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyVideoDomainInfoReadErrorV1 {
    InvalidVideoId,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Deserialize)]
struct VideoAuthorityRowV1 {
    owner_id: Option<String>,
    shared_organization_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DomainRowV1 {
    custom_domain: Option<String>,
    domain_verified_iso: Option<String>,
}

pub(crate) struct D1LegacyVideoDomainInfoAuthorityV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyVideoDomainInfoAuthorityV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn read(
        &self,
        video_id: &str,
    ) -> Result<LegacyVideoDomainInfoReadV1, LegacyVideoDomainInfoReadErrorV1> {
        if !valid_legacy_identifier(video_id) {
            return Err(LegacyVideoDomainInfoReadErrorV1::InvalidVideoId);
        }
        let mut video_rows =
            query_rows::<VideoAuthorityRowV1>(self.database, VIDEO_AUTHORITY_SQL, video_id).await?;
        if video_rows.len() > 1 {
            return Err(LegacyVideoDomainInfoReadErrorV1::Corrupt);
        }
        let Some(video) = video_rows.pop() else {
            return Ok(LegacyVideoDomainInfoReadV1::VideoNotFound);
        };
        let Some(owner_id) = video
            .owner_id
            .filter(|value| valid_legacy_identifier(value))
        else {
            return Ok(LegacyVideoDomainInfoReadV1::InvalidVideoData);
        };

        let shared = match video.shared_organization_id {
            Some(organization_id) if valid_legacy_identifier(&organization_id) => {
                read_optional_domain(self.database, ORGANIZATION_DOMAIN_SQL, &organization_id)
                    .await?
            }
            Some(_) => return Err(LegacyVideoDomainInfoReadErrorV1::Corrupt),
            None => None,
        };
        if shared
            .as_ref()
            .is_some_and(LegacyVideoDomainInfoProjectionV1::is_usable)
        {
            return Ok(LegacyVideoDomainInfoReadV1::Found(
                legacy_video_domain_info_select(shared, None),
            ));
        }

        let owner = read_optional_domain(self.database, OWNER_DOMAIN_SQL, &owner_id).await?;
        Ok(LegacyVideoDomainInfoReadV1::Found(
            legacy_video_domain_info_select(shared, owner),
        ))
    }
}

async fn read_optional_domain(
    database: &D1Database,
    query: &str,
    identifier: &str,
) -> Result<Option<LegacyVideoDomainInfoProjectionV1>, LegacyVideoDomainInfoReadErrorV1> {
    let mut rows = query_rows::<DomainRowV1>(database, query, identifier).await?;
    if rows.len() > 1 {
        return Err(LegacyVideoDomainInfoReadErrorV1::Corrupt);
    }
    rows.pop().map(decode_domain).transpose()
}

async fn query_rows<T>(
    database: &D1Database,
    query: &str,
    identifier: &str,
) -> Result<Vec<T>, LegacyVideoDomainInfoReadErrorV1>
where
    T: for<'de> Deserialize<'de>,
{
    let result = database
        .prepare(query)
        .bind(&[JsValue::from_str(identifier)])
        .map_err(|_| LegacyVideoDomainInfoReadErrorV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyVideoDomainInfoReadErrorV1::Unavailable)?;
    if !result.success() {
        return Err(LegacyVideoDomainInfoReadErrorV1::Unavailable);
    }
    result
        .results::<T>()
        .map_err(|_| LegacyVideoDomainInfoReadErrorV1::Corrupt)
}

fn decode_domain(
    row: DomainRowV1,
) -> Result<LegacyVideoDomainInfoProjectionV1, LegacyVideoDomainInfoReadErrorV1> {
    if row
        .custom_domain
        .as_ref()
        .is_some_and(|value| value.len() > MAX_CUSTOM_DOMAIN_BYTES)
        || row
            .domain_verified_iso
            .as_deref()
            .is_some_and(|value| !valid_iso_timestamp(value))
    {
        return Err(LegacyVideoDomainInfoReadErrorV1::Corrupt);
    }
    Ok(LegacyVideoDomainInfoProjectionV1 {
        custom_domain: row.custom_domain,
        domain_verified_iso: row.domain_verified_iso,
    })
}

fn valid_legacy_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LEGACY_ID_BYTES
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_iso_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 24
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes[23] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) || byte.is_ascii_digit()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_preserve_unordered_first_row_precedence() {
        assert!(VIDEO_AUTHORITY_SQL.contains("LIMIT 1"));
        assert!(!VIDEO_AUTHORITY_SQL.contains("ORDER BY"));
        assert!(ORGANIZATION_DOMAIN_SQL.contains("organization.id = ?1"));
        assert!(OWNER_DOMAIN_SQL.contains("organization.owner_id = ?1"));
        assert!(!OWNER_DOMAIN_SQL.contains("ORDER BY"));
    }

    #[test]
    fn timestamp_and_identifier_bounds_fail_closed() {
        assert!(valid_legacy_identifier("0123456789abcde"));
        assert!(!valid_legacy_identifier(""));
        assert!(!valid_legacy_identifier("bad\nvalue"));
        assert!(valid_iso_timestamp("2026-07-17T19:00:00.000Z"));
        assert!(!valid_iso_timestamp("true"));
    }

    #[test]
    fn corrupt_domain_rows_are_rejected() {
        assert!(
            decode_domain(DomainRowV1 {
                custom_domain: Some("example.com".into()),
                domain_verified_iso: Some("not-a-date".into()),
            })
            .is_err()
        );
    }
}
