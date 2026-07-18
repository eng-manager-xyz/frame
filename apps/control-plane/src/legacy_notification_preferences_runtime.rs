//! Exact D1 adapter for Cap's standalone notification-preferences GET route.

use async_trait::async_trait;
use frame_application::{
    LegacyNotificationPreferencesCandidateV1, LegacyNotificationPreferencesV1,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const READ_FOR_ACTOR_SQL: &str =
    include_str!("../queries/legacy_notification_preferences/read_for_actor.sql");
const MAX_ACTOR_ID_BYTES: usize = 256;

pub(crate) const LEGACY_NOTIFICATION_PREFERENCES_PATH: &str = "/api/notifications/preferences";
pub(crate) const LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID: &str = "cap-v1-d130c840f654bd72";
pub(crate) const LEGACY_NOTIFICATION_PREFERENCES_CONTENT_TYPE: &str = "application/json";
pub(crate) const LEGACY_NOTIFICATION_PREFERENCES_UNAUTHORIZED_BODY: &str =
    r#"{"error":"Unauthorized"}"#;
pub(crate) const LEGACY_NOTIFICATION_PREFERENCES_FAILURE_BODY: &str =
    r#"{"error":"Failed to fetch user preferences"}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyNotificationPreferencesErrorV1 {
    InvalidActor,
    Unavailable,
    Corrupt,
}

#[async_trait(?Send)]
pub(crate) trait LegacyNotificationPreferencesAuthorityV1 {
    async fn read_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacyNotificationPreferencesV1, LegacyNotificationPreferencesErrorV1>;
}

pub(crate) struct D1LegacyNotificationPreferencesAuthorityV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyNotificationPreferencesAuthorityV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }
}

#[derive(Debug, Deserialize)]
struct PreferencesRow {
    preferences_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SourcePreferences {
    notifications: SourceNotifications,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceNotifications {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    #[serde(default)]
    pause_anon_views: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePreferences {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    pause_anon_views: bool,
}

#[async_trait(?Send)]
impl LegacyNotificationPreferencesAuthorityV1 for D1LegacyNotificationPreferencesAuthorityV1<'_> {
    async fn read_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacyNotificationPreferencesV1, LegacyNotificationPreferencesErrorV1> {
        if !valid_actor_id(actor_id) {
            return Err(LegacyNotificationPreferencesErrorV1::InvalidActor);
        }
        let result = self
            .database
            .prepare(READ_FOR_ACTOR_SQL)
            .bind(&[JsValue::from_str(actor_id)])
            .map_err(|_| LegacyNotificationPreferencesErrorV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyNotificationPreferencesErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyNotificationPreferencesErrorV1::Unavailable);
        }
        let rows = result
            .results::<serde_json::Value>()
            .map_err(|_| LegacyNotificationPreferencesErrorV1::Unavailable)?
            .into_iter()
            .map(|row| {
                serde_json::from_value::<PreferencesRow>(row)
                    .map_err(|_| LegacyNotificationPreferencesErrorV1::Corrupt)
            })
            .collect::<Result<Vec<_>, _>>()?;
        decode_rows(rows)
    }
}

pub(crate) fn exact_json_body(
    preferences: LegacyNotificationPreferencesV1,
) -> Result<Vec<u8>, LegacyNotificationPreferencesErrorV1> {
    serde_json::to_vec(&WirePreferences {
        pause_comments: preferences.pause_comments(),
        pause_replies: preferences.pause_replies(),
        pause_views: preferences.pause_views(),
        pause_reactions: preferences.pause_reactions(),
        pause_anon_views: preferences.pause_anon_views(),
    })
    .map_err(|_| LegacyNotificationPreferencesErrorV1::Corrupt)
}

fn valid_actor_id(actor_id: &str) -> bool {
    !actor_id.is_empty()
        && actor_id.len() <= MAX_ACTOR_ID_BYTES
        && actor_id.is_ascii()
        && !actor_id.bytes().any(|byte| byte.is_ascii_control())
}

fn decode_rows(
    mut rows: Vec<PreferencesRow>,
) -> Result<LegacyNotificationPreferencesV1, LegacyNotificationPreferencesErrorV1> {
    if rows.len() > 1 {
        return Err(LegacyNotificationPreferencesErrorV1::Corrupt);
    }
    let Some(row) = rows.pop() else {
        // The pinned source defaults if its second user read races with a
        // deletion after getCurrentUser authenticated the request.
        return Ok(LegacyNotificationPreferencesV1::default());
    };
    let candidate = row
        .preferences_json
        .as_deref()
        .and_then(|source| serde_json::from_str::<SourcePreferences>(source).ok())
        .map(|source| {
            LegacyNotificationPreferencesCandidateV1::new(
                source.notifications.pause_comments,
                source.notifications.pause_replies,
                source.notifications.pause_views,
                source.notifications.pause_reactions,
                source.notifications.pause_anon_views,
            )
        });
    Ok(LegacyNotificationPreferencesV1::from_validated_source(
        candidate,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(preferences_json: Option<&str>) -> PreferencesRow {
        PreferencesRow {
            preferences_json: preferences_json.map(str::to_owned),
        }
    }

    fn body(source: Option<&str>) -> Vec<u8> {
        let preferences = decode_rows(vec![row(source)]).expect("source response");
        exact_json_body(preferences).expect("exact JSON")
    }

    #[test]
    fn query_is_actor_only_and_does_not_infer_tenant_or_user_state() {
        assert_eq!(READ_FOR_ACTOR_SQL.matches('?').count(), 1);
        for token in ["u.preferences_json", "WHERE u.id = ?1", "LIMIT 1"] {
            assert!(
                READ_FOR_ACTOR_SQL.contains(token),
                "missing SQL token: {token}"
            );
        }
        for forbidden in [
            "?2",
            "organization_members",
            "active_organization_id",
            "u.status",
            "u.deleted_at_ms",
        ] {
            assert!(
                !READ_FOR_ACTOR_SQL.contains(forbidden),
                "source handler does not impose filter: {forbidden}"
            );
        }
    }

    #[test]
    fn missing_null_and_schema_invalid_preferences_default_all_flags() {
        let default_body = br#"{"pauseComments":false,"pauseReplies":false,"pauseViews":false,"pauseReactions":false,"pauseAnonViews":false}"#;
        for source in [
            None,
            Some("null"),
            Some("{}"),
            Some(r#"{"notifications":{}}"#),
            Some(
                r#"{"notifications":{"pauseComments":"true","pauseReplies":false,"pauseViews":true,"pauseReactions":false,"pauseAnonViews":true}}"#,
            ),
            Some(
                r#"{"notifications":{"pauseComments":true,"pauseReplies":false,"pauseViews":true,"pauseReactions":false,"pauseAnonViews":null}}"#,
            ),
            Some("not-json"),
        ] {
            assert_eq!(body(source), default_body, "source: {source:?}");
        }
        assert_eq!(
            decode_rows(Vec::new()).expect("user disappeared"),
            LegacyNotificationPreferencesV1::default()
        );
    }

    #[test]
    fn valid_shape_defaults_only_missing_anon_and_strips_unknown_fields() {
        assert_eq!(
            body(Some(
                r#"{"trackedEvents":{"user_signed_up":"ignored"},"notifications":{"pauseComments":true,"pauseReplies":false,"pauseViews":true,"pauseReactions":false,"unknown":{"bad":true}}}"#,
            )),
            br#"{"pauseComments":true,"pauseReplies":false,"pauseViews":true,"pauseReactions":false,"pauseAnonViews":false}"#
        );
        assert_eq!(
            body(Some(
                r#"{"notifications":{"pauseComments":false,"pauseReplies":true,"pauseViews":false,"pauseReactions":true,"pauseAnonViews":true}}"#,
            )),
            br#"{"pauseComments":false,"pauseReplies":true,"pauseViews":false,"pauseReactions":true,"pauseAnonViews":true}"#
        );
    }

    #[test]
    fn exact_explicit_response_bodies_and_media_type_are_frozen() {
        assert_eq!(
            LEGACY_NOTIFICATION_PREFERENCES_PATH,
            "/api/notifications/preferences"
        );
        assert_eq!(
            LEGACY_NOTIFICATION_PREFERENCES_CONTENT_TYPE,
            "application/json"
        );
        assert_eq!(
            LEGACY_NOTIFICATION_PREFERENCES_UNAUTHORIZED_BODY,
            r#"{"error":"Unauthorized"}"#
        );
        assert_eq!(
            LEGACY_NOTIFICATION_PREFERENCES_FAILURE_BODY,
            r#"{"error":"Failed to fetch user preferences"}"#
        );
    }

    #[test]
    fn ambiguous_authority_rows_and_invalid_actor_ids_fail_closed() {
        assert_eq!(
            decode_rows(vec![row(None), row(None)]),
            Err(LegacyNotificationPreferencesErrorV1::Corrupt)
        );
        for actor_id in ["", "bad\nactor"] {
            assert!(!valid_actor_id(actor_id));
        }
        assert!(!valid_actor_id(&"a".repeat(MAX_ACTOR_ID_BYTES + 1)));
        assert!(valid_actor_id("user_01HX8Z9Q7Q33C46P4T0W0M8YQF"));
    }
}
