//! D1 authority and durable provider intent staging for Cap analytics.
//!
//! No Tinybird count or provider success is synthesized here. Provider-bound
//! operations stop in a durable `pending` state; the shared compatibility gate
//! remains `provider_execution` until a real executor and response verifier are
//! integrated.

use async_trait::async_trait;
use frame_application::{
    LEGACY_ANALYTICS_ANON_NOTIFICATION_CUTOFF_MS, LEGACY_ANALYTICS_VIEW_DELAY_MS,
    LegacyAnalyticsCommandInputV1, LegacyAnalyticsCommandV1, LegacyAnalyticsDashboardRangeV1,
    LegacyAnalyticsOutcomeV1, LegacyAnalyticsPortErrorV1, LegacyAnalyticsPortV1,
    LegacyAnalyticsResultV1, LegacyAnalyticsTrackEventV1, analytics_session_digest,
    anonymous_viewer_name,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const VIDEO_AUTHORITY_SQL: &str = include_str!("../queries/legacy_analytics/video_authority.sql");
const DASHBOARD_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_analytics/dashboard_authority.sql");
const TRACK_VIDEO_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_analytics/track_video_snapshot.sql");
const ACTOR_PROFILE_SQL: &str = include_str!("../queries/legacy_analytics/actor_profile.sql");
const OPERATION_READ_SQL: &str = include_str!("../queries/legacy_analytics/operation_read.sql");
const OPERATION_CLAIM_SQL: &str = include_str!("../queries/legacy_analytics/operation_claim.sql");
const QUERY_OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_analytics/query_outbox_insert.sql");
const EVENT_OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_analytics/event_outbox_insert.sql");
const EMAIL_OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_analytics/email_outbox_insert.sql");
const NOTIFICATION_OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_analytics/notification_outbox_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_analytics/audit_insert.sql");
const SIGNUP_CLAIM_SQL: &str = include_str!("../queries/legacy_analytics/signup_claim.sql");
const PASSWORD_GRANT_ISSUE_SQL: &str =
    include_str!("../queries/legacy_analytics/password_grant_issue.sql");

const OPERATION_UNIQUE: &str = "UNIQUE constraint failed: legacy_analytics_provider_operations_v1";

type PortResult<T> = Result<T, LegacyAnalyticsPortErrorV1>;

#[derive(Debug, Deserialize)]
struct VideoAuthorityRow {
    native_video_id: String,
    #[serde(rename = "owner_id")]
    _owner_id: String,
    organization_id: Option<String>,
    legacy_public: i64,
    actor_is_owner: i64,
    actor_has_organization_share: i64,
    actor_has_space_share: i64,
    password_required: i64,
    password_granted: i64,
    email_allowed: i64,
}

impl VideoAuthorityRow {
    fn can_view(&self) -> bool {
        if self.actor_is_owner == 1 {
            return true;
        }
        let password_allowed = self.password_required == 0 || self.password_granted == 1;
        if self.actor_has_organization_share == 1 || self.actor_has_space_share == 1 {
            return password_allowed;
        }
        self.legacy_public == 1 && self.email_allowed == 1 && password_allowed
    }
}

#[derive(Debug, Deserialize)]
struct DashboardAuthorityRow {
    actor_id: String,
    active_organization_id: Option<String>,
    organization_id: String,
    organization_allowed: i64,
    space_allowed: i64,
    video_allowed: i64,
    lifetime_start_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TrackVideoRow {
    native_video_id: String,
    owner_id: String,
    #[serde(rename = "organization_id")]
    _organization_id: Option<String>,
    video_name: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    has_active_upload: i64,
    first_view_email_claimed: i64,
    owner_email: String,
    owner_name: String,
    owner_active_organization_id: Option<String>,
    pause_views: i64,
    pause_anonymous_views: i64,
}

#[derive(Debug, Deserialize)]
struct ActorProfileRow {
    id: String,
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
}

pub struct D1LegacyAnalyticsPortV1<'a> {
    database: &'a D1Database,
}

impl<'a> D1LegacyAnalyticsPortV1<'a> {
    #[must_use]
    pub const fn new(database: &'a D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn issue_password_grant(
        &self,
        requested_video_ids: &[String],
        verified_password_hashes: &[String],
        grant_digest: &str,
        now_ms: i64,
    ) -> PortResult<()> {
        if requested_video_ids.is_empty() || verified_password_hashes.is_empty() {
            return Ok(());
        }
        let expires_at_ms = now_ms
            .checked_add(5 * 60 * 1_000)
            .filter(|value| *value <= 9_007_199_254_740_991)
            .ok_or(LegacyAnalyticsPortErrorV1::Corrupt)?;
        let requested = serde_json::to_string(requested_video_ids)
            .map_err(|_| LegacyAnalyticsPortErrorV1::Corrupt)?;
        let candidates = serde_json::to_string(verified_password_hashes)
            .map_err(|_| LegacyAnalyticsPortErrorV1::Corrupt)?;
        let result = self
            .statement(
                PASSWORD_GRANT_ISSUE_SQL,
                vec![
                    text(&requested),
                    text(&candidates),
                    text(grant_digest),
                    number(now_ms),
                    number(expires_at_ms),
                ],
            )?
            .run()
            .into_send()
            .await
            .map_err(|error| map_d1(&error.to_string()))?;
        if result.success() {
            Ok(())
        } else {
            Err(map_d1(result.error().as_deref().unwrap_or_default()))
        }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyAnalyticsPortErrorV1::Unavailable)
    }

    async fn rows<T>(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyAnalyticsPortErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyAnalyticsPortErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> PortResult<Vec<D1Result>> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyAnalyticsPortErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1(failed.error().as_deref().unwrap_or_default()));
        }
        Ok(results)
    }

    async fn prior_operation(
        &self,
        command: &LegacyAnalyticsCommandV1,
    ) -> PortResult<Option<OperationRow>> {
        let execution = command
            .execution_key_digest()
            .ok_or(LegacyAnalyticsPortErrorV1::Corrupt)?;
        self.rows::<OperationRow>(
            OPERATION_READ_SQL,
            vec![
                text(command.input().surface().operation_id()),
                text(command.principal_digest()),
                text(&execution),
            ],
        )
        .await
        .map(|rows| rows.into_iter().next())
    }

    fn operation_claim(
        &self,
        command: &LegacyAnalyticsCommandV1,
        target_video_id: Option<&str>,
        operation_kind: &str,
    ) -> PortResult<D1PreparedStatement> {
        let operation_id = operation_id(command)?;
        let execution = command
            .execution_key_digest()
            .ok_or(LegacyAnalyticsPortErrorV1::Corrupt)?;
        self.statement(
            OPERATION_CLAIM_SQL,
            vec![
                text(&operation_id),
                text(command.input().surface().operation_id()),
                text(operation_kind),
                text(command.principal_digest()),
                optional_text(command.actor_id()),
                optional_text(command.active_organization_id()),
                optional_text(target_video_id),
                text(&execution),
                text(command.request_digest()),
                number(command.now_ms()),
            ],
        )
    }

    async fn video_authority(
        &self,
        command: &LegacyAnalyticsCommandV1,
        video_id: &str,
    ) -> PortResult<VideoAuthorityRow> {
        let row = self
            .rows::<VideoAuthorityRow>(
                VIDEO_AUTHORITY_SQL,
                vec![
                    text(video_id),
                    optional_text(command.actor_id()),
                    optional_text(command.password_grant_digest()),
                    number(command.now_ms()),
                ],
            )
            .await?
            .into_iter()
            .next()
            .ok_or(LegacyAnalyticsPortErrorV1::NotFound)?;
        if !row.can_view() {
            return Err(LegacyAnalyticsPortErrorV1::NotFound);
        }
        Ok(row)
    }

    async fn stage_query(
        &self,
        command: &LegacyAnalyticsCommandV1,
        query_kind: &str,
        target_video_id: Option<&str>,
        request: Value,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        if let Some(prior) = self.prior_operation(command).await? {
            if prior.request_digest != command.request_digest() {
                return Err(LegacyAnalyticsPortErrorV1::Conflict);
            }
            if prior.state != "pending" {
                return Err(LegacyAnalyticsPortErrorV1::Corrupt);
            }
            return Ok(provider_pending(prior.operation_id, true));
        }
        let operation_id = operation_id(command)?;
        let request_json =
            serde_json::to_string(&request).map_err(|_| LegacyAnalyticsPortErrorV1::Corrupt)?;
        let target_digest = digest(target_video_id.unwrap_or("dashboard"));
        self.batch(vec![
            self.operation_claim(command, target_video_id, "query")?,
            self.statement(
                QUERY_OUTBOX_INSERT_SQL,
                vec![
                    text(&operation_id),
                    text(query_kind),
                    text(&request_json),
                    text(command.request_digest()),
                    number(command.now_ms()),
                ],
            )?,
            self.audit_statement(
                command,
                Some(&operation_id),
                &target_digest,
                "provider_query_pending",
            )?,
        ])
        .await?;
        Ok(provider_pending(operation_id, false))
    }

    fn audit_statement(
        &self,
        command: &LegacyAnalyticsCommandV1,
        operation_id: Option<&str>,
        target_digest: &str,
        result_kind: &str,
    ) -> PortResult<D1PreparedStatement> {
        self.statement(
            AUDIT_INSERT_SQL,
            vec![
                text(&Uuid::now_v7().to_string()),
                optional_text(operation_id),
                text(command.input().surface().operation_id()),
                text(command.principal_digest()),
                text(target_digest),
                text(command.request_digest()),
                text(result_kind),
                number(command.now_ms()),
            ],
        )
    }

    async fn execute_video_count(
        &self,
        command: &LegacyAnalyticsCommandV1,
        video_id: &str,
        range_days: u16,
        query_kind: &str,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        let authority = self.video_authority(command, video_id).await?;
        let (from_ms, to_ms) = video_window(command.now_ms(), range_days);
        self.stage_query(
            command,
            query_kind,
            Some(video_id),
            json!({
                "requestedVideoId": video_id,
                "nativeVideoId": authority.native_video_id,
                "organizationId": authority.organization_id,
                "pathname": format!("/s/{video_id}"),
                "rangeDays": range_days,
                "fromMs": from_ms,
                "toMs": to_ms,
                "aggregateThenRawFallback": true,
                "failurePolicy": "fail_closed",
            }),
        )
        .await
    }

    async fn execute_dashboard(
        &self,
        command: &LegacyAnalyticsCommandV1,
        requested_organization_id: Option<&str>,
        space_id: Option<&str>,
        video_id: Option<&str>,
        range: LegacyAnalyticsDashboardRangeV1,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        let actor_id = command
            .actor_id()
            .ok_or(LegacyAnalyticsPortErrorV1::Unauthorized)?;
        let active = command
            .active_organization_id()
            .ok_or(LegacyAnalyticsPortErrorV1::Forbidden)?;
        if requested_organization_id.is_some_and(|requested| requested != active) {
            return Err(LegacyAnalyticsPortErrorV1::Forbidden);
        }
        let row = self
            .rows::<DashboardAuthorityRow>(
                DASHBOARD_AUTHORITY_SQL,
                vec![
                    text(actor_id),
                    text(active),
                    optional_text(space_id),
                    optional_text(video_id),
                ],
            )
            .await?
            .into_iter()
            .next()
            .ok_or(LegacyAnalyticsPortErrorV1::Forbidden)?;
        if row.active_organization_id.as_deref() != Some(active)
            || row.organization_id != active
            || row.organization_allowed != 1
        {
            return Err(LegacyAnalyticsPortErrorV1::Forbidden);
        }
        if row.space_allowed != 1 || row.video_allowed != 1 {
            return Err(LegacyAnalyticsPortErrorV1::NotFound);
        }
        let window = dashboard_window(range, command.now_ms(), row.lifetime_start_ms);
        self.stage_query(
            command,
            "dashboard",
            video_id,
            json!({
                "actorId": row.actor_id,
                "organizationId": active,
                "spaceId": space_id,
                "videoId": video_id,
                "range": range.source_value(),
                "fromMs": window.from_ms,
                "toMs": window.to_ms,
                "bucket": window.bucket,
                "aggregateThenRawFallback": true,
                "combineWithD1CapsCommentsReactions": true,
                "failurePolicy": "fail_closed",
            }),
        )
        .await
    }

    async fn execute_bulk(
        &self,
        command: &LegacyAnalyticsCommandV1,
        video_ids: &[String],
        range_days: u16,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        let mut entries = Vec::with_capacity(video_ids.len());
        for (index, video_id) in video_ids.iter().enumerate() {
            match self.video_authority(command, video_id).await {
                Ok(authority) => entries.push(json!({
                    "index": index,
                    "requestedVideoId": video_id,
                    "nativeVideoId": authority.native_video_id,
                    "organizationId": authority.organization_id,
                    "pathname": format!("/s/{video_id}"),
                    "authorized": true,
                })),
                Err(LegacyAnalyticsPortErrorV1::NotFound) => entries.push(json!({
                    "index": index,
                    "requestedVideoId": video_id,
                    "authorized": false,
                    "failure": "not_found",
                })),
                Err(error) => return Err(error),
            }
        }
        let (from_ms, to_ms) = video_window(command.now_ms(), range_days);
        self.stage_query(
            command,
            "video_count_bulk",
            None,
            json!({
                "entries": entries,
                "rangeDays": range_days,
                "fromMs": from_ms,
                "toMs": to_ms,
                "maxItems": 50,
                "concurrency": 10,
                "aggregateThenRawFallback": true,
                "failurePolicy": "per_item_non_disclosure",
            }),
        )
        .await
    }

    async fn execute_track(
        &self,
        command: &LegacyAnalyticsCommandV1,
        event: &LegacyAnalyticsTrackEventV1,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        if let Some(prior) = self.prior_operation(command).await? {
            if prior.request_digest != command.request_digest() {
                return Err(LegacyAnalyticsPortErrorV1::Conflict);
            }
            return Ok(provider_pending(prior.operation_id, true));
        }
        // Cap deliberately treats a failed/missing video lookup as an absent
        // snapshot and still submits the page hit. The durable event insert
        // remains authoritative; only the optional owner/email/notification
        // enrichment is omitted when this read cannot be completed.
        let snapshot = self
            .rows::<TrackVideoRow>(TRACK_VIDEO_SNAPSHOT_SQL, vec![text(&event.video_id)])
            .await
            .unwrap_or_default()
            .into_iter()
            .next();
        if snapshot.as_ref().is_some_and(|video| {
            command.actor_id() == Some(video.owner_id.as_str())
                || video.has_active_upload == 1
                || command.now_ms() - video.updated_at_ms < LEGACY_ANALYTICS_VIEW_DELAY_MS
        }) {
            return Ok(LegacyAnalyticsOutcomeV1 {
                result: LegacyAnalyticsResultV1::TrackSkipped,
                replayed: false,
            });
        }

        let tenant_id = event
            .requested_organization_id
            .clone()
            .or_else(|| snapshot.as_ref().map(|video| video.owner_id.clone()))
            .or_else(|| event.requested_owner_id.clone())
            .or_else(|| (!event.hostname.is_empty()).then(|| format!("domain:{}", event.hostname)))
            .unwrap_or_else(|| "public".into());
        let operation_id = operation_id(command)?;
        let session_id = event.session_id.as_deref().unwrap_or("anon");
        let provider_event = json!({
            "timestamp": event.timestamp,
            "session_id": session_id,
            "action": "page_hit",
            "version": "1.0",
            "tenant_id": tenant_id,
            "video_id": event.video_id,
            "pathname": event.pathname,
            "country": event.country,
            "region": event.region,
            "city": event.city,
            "browser": event.user_agent.browser,
            "device": event.user_agent.device,
            "os": event.user_agent.operating_system,
            "user_id": command.actor_id(),
        });
        let event_json = serde_json::to_string(&provider_event)
            .map_err(|_| LegacyAnalyticsPortErrorV1::Corrupt)?;
        let mut statements = vec![
            self.operation_claim(command, Some(&event.video_id), "event")?,
            self.statement(
                EVENT_OUTBOX_INSERT_SQL,
                vec![
                    text(&operation_id),
                    text(&event.timestamp),
                    text(session_id),
                    text(&tenant_id),
                    text(&event.video_id),
                    text(&event.pathname),
                    text(&event.country),
                    text(&event.region),
                    text(&event.city),
                    text(&event.user_agent.browser),
                    text(&event.user_agent.device),
                    text(&event.user_agent.operating_system),
                    text(&event.user_agent.raw),
                    optional_text(command.actor_id()),
                    text(&event_json),
                    number(command.now_ms()),
                ],
            )?,
        ];

        if let Some(video) = snapshot.as_ref() {
            let is_new_video = video.created_at_ms >= LEGACY_ANALYTICS_ANON_NOTIFICATION_CUTOFF_MS;
            let first_view = is_new_video && video.first_view_email_claimed == 0;
            let actor_profile = if let Some(actor_id) = command.actor_id() {
                self.rows::<ActorProfileRow>(ACTOR_PROFILE_SQL, vec![text(actor_id)])
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .next()
            } else {
                None
            };
            if first_view {
                if let Some(viewer) = actor_profile.as_ref().filter(|_| video.pause_views == 0) {
                    statements.push(self.email_statement(
                        &operation_id,
                        video,
                        &event.video_id,
                        Some(viewer),
                        &viewer.display_name,
                        false,
                        command.now_ms(),
                    )?);
                } else if command.actor_id().is_none() && video.pause_anonymous_views == 0 {
                    let viewer_name = event
                        .session_id
                        .as_deref()
                        .map_or_else(|| "Anonymous Viewer".into(), anonymous_viewer_name);
                    statements.push(self.email_statement(
                        &operation_id,
                        video,
                        &event.video_id,
                        None,
                        &viewer_name,
                        true,
                        command.now_ms(),
                    )?);
                }
            }
            if command.actor_id().is_none()
                && is_new_video
                && video.pause_anonymous_views == 0
                && let (Some(session), Some(organization_id)) = (
                    event.session_id.as_deref(),
                    video.owner_active_organization_id.as_deref(),
                )
            {
                statements.push(self.notification_statement(
                    &operation_id,
                    video,
                    organization_id,
                    session,
                    event,
                    command.now_ms(),
                )?);
            }
        }
        statements.push(self.audit_statement(
            command,
            Some(&operation_id),
            &digest(&event.video_id),
            "provider_event_pending",
        )?);
        self.batch(statements).await?;
        Ok(provider_pending(operation_id, false))
    }

    #[allow(clippy::too_many_arguments)]
    fn email_statement(
        &self,
        operation_id: &str,
        video: &TrackVideoRow,
        requested_video_id: &str,
        viewer: Option<&ActorProfileRow>,
        viewer_name: &str,
        is_anonymous: bool,
        now_ms: i64,
    ) -> PortResult<D1PreparedStatement> {
        let video_name = if video.video_name.is_empty() {
            "Untitled Video"
        } else {
            &video.video_name
        };
        let payload = json!({
            "videoId": requested_video_id,
            "videoName": video_name,
            "ownerName": video.owner_name,
            "recipientEmail": video.owner_email,
            "viewerName": viewer_name,
            "isAnonymous": is_anonymous,
        });
        self.statement(
            EMAIL_OUTBOX_INSERT_SQL,
            vec![
                text(operation_id),
                text(&video.native_video_id),
                text(&video.owner_id),
                text(&video.owner_email),
                optional_text(viewer.map(|profile| profile.id.as_str())),
                text(viewer_name),
                number(i64::from(is_anonymous)),
                text(&payload.to_string()),
                number(now_ms),
            ],
        )
    }

    fn notification_statement(
        &self,
        operation_id: &str,
        video: &TrackVideoRow,
        organization_id: &str,
        session_id: &str,
        event: &LegacyAnalyticsTrackEventV1,
        now_ms: i64,
    ) -> PortResult<D1PreparedStatement> {
        let session_hash = analytics_session_digest(session_id);
        let anonymous_name = anonymous_viewer_name(session_id);
        let location = match (event.city.as_str(), event.country.as_str()) {
            ("", "") => None,
            (city, "") => Some(city.to_owned()),
            ("", country) => Some(country.to_owned()),
            (city, country) => Some(format!("{city}, {country}")),
        };
        let payload = json!({
            "videoId": event.video_id,
            "anonymousName": anonymous_name,
            "location": location,
        });
        self.statement(
            NOTIFICATION_OUTBOX_INSERT_SQL,
            vec![
                text(operation_id),
                text(&format!("anon_view:{}:{session_hash}", event.video_id)),
                text(&video.native_video_id),
                text(organization_id),
                text(&video.owner_id),
                text(&anonymous_name),
                optional_text(location.as_deref()),
                text(&payload.to_string()),
                number(now_ms),
            ],
        )
    }

    async fn execute_signup(
        &self,
        command: &LegacyAnalyticsCommandV1,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        let Some(actor_id) = command.actor_id() else {
            return Ok(signup(false));
        };
        let result = match self
            .statement(
                SIGNUP_CLAIM_SQL,
                vec![text(actor_id), number(command.now_ms())],
            )?
            .run()
            .into_send()
            .await
        {
            Ok(result) if result.success() => result,
            _ => return Ok(signup(false)),
        };
        let changed = result
            .meta()
            .ok()
            .flatten()
            .and_then(|meta| meta.changes)
            .unwrap_or(0);
        Ok(signup(changed > 0))
    }
}

#[async_trait(?Send)]
impl LegacyAnalyticsPortV1 for D1LegacyAnalyticsPortV1<'_> {
    async fn execute(
        &self,
        command: LegacyAnalyticsCommandV1,
    ) -> PortResult<LegacyAnalyticsOutcomeV1> {
        match command.input().clone() {
            LegacyAnalyticsCommandInputV1::VideoCount {
                video_id,
                range_days,
            } => {
                self.execute_video_count(&command, &video_id, range_days, "video_count_route")
                    .await
            }
            LegacyAnalyticsCommandInputV1::Track(event) => {
                self.execute_track(&command, &event).await
            }
            LegacyAnalyticsCommandInputV1::Dashboard {
                requested_organization_id,
                space_id,
                video_id,
                range,
            } => {
                self.execute_dashboard(
                    &command,
                    requested_organization_id.as_deref(),
                    space_id.as_deref(),
                    video_id.as_deref(),
                    range,
                )
                .await
            }
            LegacyAnalyticsCommandInputV1::VideoHttp {
                video_id,
                range_days,
            } => {
                self.execute_video_count(&command, &video_id, range_days, "video_count_http")
                    .await
            }
            LegacyAnalyticsCommandInputV1::VideoRpc {
                video_ids,
                range_days,
            } => self.execute_bulk(&command, &video_ids, range_days).await,
            LegacyAnalyticsCommandInputV1::Signup => self.execute_signup(&command).await,
            LegacyAnalyticsCommandInputV1::VideoAction {
                video_id,
                range_days,
            } => {
                self.execute_video_count(&command, &video_id, range_days, "video_count_action")
                    .await
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardWindowV1 {
    from_ms: i64,
    to_ms: i64,
    bucket: &'static str,
}

fn dashboard_window(
    range: LegacyAnalyticsDashboardRangeV1,
    now_ms: i64,
    lifetime_start_ms: Option<i64>,
) -> DashboardWindowV1 {
    const HOUR: i64 = 60 * 60 * 1_000;
    const DAY: i64 = 24 * HOUR;
    let (from_ms, bucket) = match range {
        LegacyAnalyticsDashboardRangeV1::Hours24 => (now_ms - 24 * HOUR, "hour"),
        LegacyAnalyticsDashboardRangeV1::Days7 => (now_ms - 7 * DAY, "day"),
        LegacyAnalyticsDashboardRangeV1::Days30 => (now_ms - 30 * DAY, "day"),
        LegacyAnalyticsDashboardRangeV1::Lifetime => (
            lifetime_start_ms
                .filter(|start| *start < now_ms)
                .unwrap_or(now_ms - 30 * DAY),
            "day",
        ),
    };
    DashboardWindowV1 {
        from_ms,
        to_ms: now_ms,
        bucket,
    }
}

fn video_window(now_ms: i64, range_days: u16) -> (i64, i64) {
    let days = i64::from(range_days.max(1));
    (now_ms - days * 24 * 60 * 60 * 1_000, now_ms)
}

fn provider_pending(operation_id: String, replayed: bool) -> LegacyAnalyticsOutcomeV1 {
    LegacyAnalyticsOutcomeV1 {
        result: LegacyAnalyticsResultV1::ProviderPending { operation_id },
        replayed,
    }
}

fn signup(should_track: bool) -> LegacyAnalyticsOutcomeV1 {
    LegacyAnalyticsOutcomeV1 {
        result: LegacyAnalyticsResultV1::SignupTracking { should_track },
        replayed: false,
    }
}

fn operation_id(command: &LegacyAnalyticsCommandV1) -> PortResult<String> {
    command
        .operation_id()
        .map(|value| value.to_string())
        .ok_or(LegacyAnalyticsPortErrorV1::Corrupt)
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn optional_text(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn map_d1(message: &str) -> LegacyAnalyticsPortErrorV1 {
    if message.contains(OPERATION_UNIQUE) {
        LegacyAnalyticsPortErrorV1::Conflict
    } else {
        LegacyAnalyticsPortErrorV1::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame_application::{
        LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS, LEGACY_ANALYTICS_SIGNUP_OPERATION_ID,
        LEGACY_ANALYTICS_TRACK_OPERATION_ID,
    };

    #[test]
    fn video_policy_preserves_owner_membership_password_and_public_order() {
        let mut row = VideoAuthorityRow {
            native_video_id: "video".into(),
            _owner_id: "owner".into(),
            organization_id: Some("org".into()),
            legacy_public: 0,
            actor_is_owner: 1,
            actor_has_organization_share: 0,
            actor_has_space_share: 0,
            password_required: 1,
            password_granted: 0,
            email_allowed: 0,
        };
        assert!(row.can_view());
        row.actor_is_owner = 0;
        row.actor_has_space_share = 1;
        assert!(!row.can_view());
        row.password_granted = 1;
        assert!(row.can_view());
        row.actor_has_space_share = 0;
        row.legacy_public = 1;
        assert!(!row.can_view());
        row.email_allowed = 1;
        assert!(row.can_view());
    }

    #[test]
    fn dashboard_and_video_windows_preserve_source_ranges() {
        let now = 1_800_000_000_000;
        assert_eq!(
            dashboard_window(LegacyAnalyticsDashboardRangeV1::Hours24, now, None),
            DashboardWindowV1 {
                from_ms: now - 86_400_000,
                to_ms: now,
                bucket: "hour",
            }
        );
        assert_eq!(
            dashboard_window(
                LegacyAnalyticsDashboardRangeV1::Lifetime,
                now,
                Some(now - 100_000)
            )
            .from_ms,
            now - 100_000
        );
        assert_eq!(
            video_window(now, LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS).1,
            now
        );
        assert_eq!(
            video_window(now, LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS).0,
            now - 90 * 86_400_000
        );
    }

    #[test]
    fn provider_sources_never_return_fabricated_counts() {
        let pending = provider_pending("operation".into(), false);
        assert_eq!(
            pending.result,
            LegacyAnalyticsResultV1::ProviderPending {
                operation_id: "operation".into()
            }
        );
        assert_eq!(
            LEGACY_ANALYTICS_TRACK_OPERATION_ID,
            "cap-v1-51dc2aa9f19a48cc"
        );
        assert_eq!(
            LEGACY_ANALYTICS_SIGNUP_OPERATION_ID,
            "cap-v1-dd88ded400188c1e"
        );
    }
}
