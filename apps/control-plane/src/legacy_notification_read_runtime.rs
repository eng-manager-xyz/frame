//! Exact D1 projection for Cap's authenticated `GET /api/notifications`.

use frame_application::{
    LegacyNotificationAuthorV1, LegacyNotificationCommentV1, LegacyNotificationKindV1,
    LegacyNotificationPayloadV1, LegacyNotificationProjectionV1, LegacyNotificationReadResultV1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const READ_ROWS_SQL: &str = include_str!("../queries/legacy_notification_read/read_rows.sql");
const READ_COUNTS_SQL: &str = include_str!("../queries/legacy_notification_read/read_counts.sql");
const MAX_ACTOR_ID_BYTES: usize = 256;
const MAX_MYSQL_DATETIME_MS: i64 = 253_402_300_799_999;

pub(crate) const LEGACY_NOTIFICATION_READ_SUCCESS_CONTENT_TYPE: &str = "application/json";
pub(crate) const LEGACY_NOTIFICATION_READ_UNAUTHORIZED_CONTENT_TYPE: &str =
    "text/plain;charset=UTF-8";
pub(crate) const LEGACY_NOTIFICATION_READ_UNAUTHORIZED_BODY: &str = r#"{"error":"Unauthorized"}"#;
pub(crate) const LEGACY_NOTIFICATION_READ_FAILURE_BODY: &str =
    r#"{"error":"Failed to fetch notifications"}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyNotificationReadErrorV1 {
    InvalidActor,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Deserialize)]
struct NotificationRow {
    id: String,
    notification_type: String,
    data_json: String,
    read_at_ms: Option<i64>,
    created_at_ms: i64,
    author_id: Option<String>,
    author_name: Option<String>,
    author_image_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    notification_type: String,
    notification_count: i64,
}

#[derive(Debug, Serialize)]
struct WireResponse {
    notifications: Vec<WireNotification>,
    count: WireCounts,
}

#[derive(Debug, Serialize)]
struct WireCounts {
    view: u64,
    comment: u64,
    reply: u64,
    reaction: u64,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum WireNotification {
    Authored(WireAuthoredNotification),
    Anonymous(WireAnonymousNotification),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAuthoredNotification {
    #[serde(rename = "type")]
    notification_type: &'static str,
    video_id: String,
    author: WireAuthor,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<WireComment>,
    id: String,
    read_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAnonymousNotification {
    #[serde(rename = "type")]
    notification_type: &'static str,
    video_id: String,
    anon_name: String,
    location: Option<String>,
    id: String,
    read_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct WireAuthor {
    id: String,
    name: String,
    avatar: Option<String>,
}

#[derive(Debug, Serialize)]
struct WireComment {
    id: String,
    content: String,
}

pub(crate) async fn read_exact_json(
    database: &D1Database,
    actor_id: &str,
) -> Result<Vec<u8>, LegacyNotificationReadErrorV1> {
    read_exact_json_with_avatar_resolver(database, actor_id, |_| None).await
}

async fn read_exact_json_with_avatar_resolver(
    database: &D1Database,
    actor_id: &str,
    mut resolve_avatar: impl FnMut(&str) -> Option<String>,
) -> Result<Vec<u8>, LegacyNotificationReadErrorV1> {
    if !valid_actor_id(actor_id) {
        return Err(LegacyNotificationReadErrorV1::InvalidActor);
    }
    let rows_result = database
        .prepare(READ_ROWS_SQL)
        .bind(&[JsValue::from_str(actor_id)])
        .map_err(|_| LegacyNotificationReadErrorV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyNotificationReadErrorV1::Unavailable)?;
    let counts_result = database
        .prepare(READ_COUNTS_SQL)
        .bind(&[JsValue::from_str(actor_id)])
        .map_err(|_| LegacyNotificationReadErrorV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyNotificationReadErrorV1::Unavailable)?;
    if !rows_result.success() || !counts_result.success() {
        return Err(LegacyNotificationReadErrorV1::Unavailable);
    }
    let rows = rows_result
        .results::<NotificationRow>()
        .map_err(|_| LegacyNotificationReadErrorV1::Corrupt)?;
    let counts = counts_result
        .results::<CountRow>()
        .map_err(|_| LegacyNotificationReadErrorV1::Corrupt)?;

    let projected = rows
        .into_iter()
        .map(|row| project_row(row, &mut resolve_avatar))
        .collect::<Vec<_>>();
    let grouped_counts = counts
        .into_iter()
        .filter_map(|row| {
            let count = u64::try_from(row.notification_count).ok()?;
            Some((parse_kind(&row.notification_type)?, count))
        })
        .collect::<Vec<_>>();
    exact_json(LegacyNotificationReadResultV1::new(
        projected,
        grouped_counts,
    ))
}

fn project_row(
    row: NotificationRow,
    resolve_avatar: &mut impl FnMut(&str) -> Option<String>,
) -> Option<LegacyNotificationProjectionV1> {
    if row.id.is_empty()
        || !valid_timestamp(row.created_at_ms)
        || row.read_at_ms.is_some_and(|value| !valid_timestamp(value))
    {
        return None;
    }
    let kind = parse_kind(&row.notification_type)?;
    let data: Value = serde_json::from_str(&row.data_json).ok()?;
    let object = data.as_object()?;
    let video_id = object.get("videoId")?.as_str()?.to_owned();
    if video_id.is_empty() {
        return None;
    }
    let payload = if kind == LegacyNotificationKindV1::AnonymousView {
        let anonymous_name = match object.get("anonName") {
            None | Some(Value::Null) => "Anonymous Viewer".to_owned(),
            Some(Value::String(value)) => value.clone(),
            Some(_) => return None,
        };
        let location = match object.get("location") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => return None,
        };
        LegacyNotificationPayloadV1::AnonymousView {
            video_id,
            anonymous_name,
            location,
        }
    } else {
        let author_id = row.author_id.filter(|value| !value.is_empty())?;
        let name = row.author_name.unwrap_or_else(|| "Unknown".to_owned());
        let resolved_avatar = row.author_image_key.as_deref().and_then(resolve_avatar);
        let comment = match kind {
            LegacyNotificationKindV1::Comment
            | LegacyNotificationKindV1::Reply
            | LegacyNotificationKindV1::Reaction => {
                let comment = object.get("comment")?.as_object()?;
                Some(LegacyNotificationCommentV1 {
                    id: comment.get("id")?.as_str()?.to_owned(),
                    content: comment.get("content")?.as_str()?.to_owned(),
                })
            }
            LegacyNotificationKindV1::View => None,
            LegacyNotificationKindV1::AnonymousView => return None,
        };
        LegacyNotificationPayloadV1::Authored {
            video_id,
            author: LegacyNotificationAuthorV1 {
                id: author_id,
                name,
                image_key: row.author_image_key,
                resolved_avatar,
            },
            comment,
        }
    };
    Some(LegacyNotificationProjectionV1 {
        id: row.id,
        kind,
        payload,
        read_at_ms: row.read_at_ms,
        created_at_ms: row.created_at_ms,
    })
}

fn exact_json(
    result: LegacyNotificationReadResultV1,
) -> Result<Vec<u8>, LegacyNotificationReadErrorV1> {
    let notifications = result
        .notifications
        .into_iter()
        .map(wire_notification)
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_vec(&WireResponse {
        notifications,
        count: WireCounts {
            view: result.counts.view,
            comment: result.counts.comment,
            reply: result.counts.reply,
            reaction: result.counts.reaction,
        },
    })
    .map_err(|_| LegacyNotificationReadErrorV1::Corrupt)
}

fn wire_notification(
    projection: LegacyNotificationProjectionV1,
) -> Result<WireNotification, LegacyNotificationReadErrorV1> {
    let read_at = projection.read_at_ms.map(iso_timestamp).transpose()?;
    let created_at = iso_timestamp(projection.created_at_ms)?;
    match projection.payload {
        LegacyNotificationPayloadV1::Authored {
            video_id,
            author,
            comment,
        } => Ok(WireNotification::Authored(WireAuthoredNotification {
            notification_type: projection.kind.wire_name(),
            video_id,
            author: WireAuthor {
                id: author.id,
                name: author.name,
                avatar: author.resolved_avatar,
            },
            comment: comment.map(|value| WireComment {
                id: value.id,
                content: value.content,
            }),
            id: projection.id,
            read_at,
            created_at,
        })),
        LegacyNotificationPayloadV1::AnonymousView {
            video_id,
            anonymous_name,
            location,
        } => Ok(WireNotification::Anonymous(WireAnonymousNotification {
            notification_type: projection.kind.wire_name(),
            video_id,
            anon_name: anonymous_name,
            location,
            id: projection.id,
            read_at,
            created_at,
        })),
    }
}

fn parse_kind(value: &str) -> Option<LegacyNotificationKindV1> {
    match value {
        "view" => Some(LegacyNotificationKindV1::View),
        "comment" => Some(LegacyNotificationKindV1::Comment),
        "reply" => Some(LegacyNotificationKindV1::Reply),
        "reaction" => Some(LegacyNotificationKindV1::Reaction),
        "anon_view" => Some(LegacyNotificationKindV1::AnonymousView),
        _ => None,
    }
}

fn valid_actor_id(actor_id: &str) -> bool {
    !actor_id.is_empty()
        && actor_id.len() <= MAX_ACTOR_ID_BYTES
        && actor_id.is_ascii()
        && !actor_id.bytes().any(|byte| byte.is_ascii_control())
}

const fn valid_timestamp(value: i64) -> bool {
    value >= 0 && value <= MAX_MYSQL_DATETIME_MS
}

fn iso_timestamp(value: i64) -> Result<String, LegacyNotificationReadErrorV1> {
    if !valid_timestamp(value) {
        return Err(LegacyNotificationReadErrorV1::Corrupt);
    }
    let total_seconds = value / 1_000;
    let milliseconds = value % 1_000;
    let days = total_seconds / 86_400;
    let seconds = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z"
    ))
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, data_json: &str) -> NotificationRow {
        NotificationRow {
            id: "notification-1".into(),
            notification_type: kind.into(),
            data_json: data_json.into(),
            read_at_ms: None,
            created_at_ms: 1_735_689_600_123,
            author_id: Some("author-1".into()),
            author_name: None,
            author_image_key: Some("profile.png".into()),
        }
    }

    #[test]
    fn authored_rows_match_zod_projection_and_avatar_failure_is_null() {
        let projected = project_row(
            row(
                "comment",
                r#"{"videoId":"video-1","authorId":"author-1","comment":{"id":"comment-1","content":"hello","ignored":true}}"#,
            ),
            &mut |_| None,
        )
        .expect("projection");
        let body = exact_json(LegacyNotificationReadResultV1::new(
            [Some(projected)],
            [(LegacyNotificationKindV1::Comment, 1)],
        ))
        .expect("json");
        assert_eq!(
            String::from_utf8(body).expect("utf8"),
            r#"{"notifications":[{"type":"comment","videoId":"video-1","author":{"id":"author-1","name":"Unknown","avatar":null},"comment":{"id":"comment-1","content":"hello"},"id":"notification-1","readAt":null,"createdAt":"2025-01-01T00:00:00.123Z"}],"count":{"view":0,"comment":1,"reply":0,"reaction":0}}"#
        );
    }

    #[test]
    fn anonymous_defaults_match_nullish_coalescing_and_counts_fold() {
        let mut anonymous = row(
            "anon_view",
            r#"{"videoId":"video-1","anonName":null,"location":null}"#,
        );
        anonymous.author_id = None;
        let projected =
            project_row(anonymous, &mut |_| Some("unused".into())).expect("anonymous projection");
        let body = exact_json(LegacyNotificationReadResultV1::new(
            [Some(projected)],
            [
                (LegacyNotificationKindV1::View, 2),
                (LegacyNotificationKindV1::AnonymousView, 3),
            ],
        ))
        .expect("json");
        let value: Value = serde_json::from_slice(&body).expect("json value");
        assert_eq!(value["notifications"][0]["anonName"], "Anonymous Viewer");
        assert_eq!(value["count"]["view"], 5);
    }

    #[test]
    fn malformed_rows_are_individually_omitted() {
        for mut invalid in [
            row("unknown", r#"{"videoId":"video"}"#),
            row("reply", r#"{"videoId":"video"}"#),
            row("view", r#"{"videoId":7}"#),
            row("anon_view", r#"{"videoId":"video","location":7}"#),
        ] {
            if invalid.notification_type == "anon_view" {
                invalid.author_id = None;
            }
            assert!(project_row(invalid, &mut |_| None).is_none());
        }
    }

    #[test]
    fn timestamp_projection_matches_javascript_date_json() {
        assert_eq!(iso_timestamp(0).expect("epoch"), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso_timestamp(951_827_696_789).expect("leap day"),
            "2000-02-29T12:34:56.789Z"
        );
        assert_eq!(
            iso_timestamp(MAX_MYSQL_DATETIME_MS).expect("mysql maximum"),
            "9999-12-31T23:59:59.999Z"
        );
    }
}
