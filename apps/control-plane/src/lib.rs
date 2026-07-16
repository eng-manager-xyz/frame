mod commands;
mod contracts;
mod routing;

use commands::{
    ApiKeyRow, COMMAND_TTL_MS, IntegrationRow, MAX_SINGLE_UPLOAD_BYTES, MediaJobResponse,
    MediaJobRow, MembershipRow, SourceObjectRow, StoredCommandRow, UploadIntentResponse, UploadRow,
    UploadStatusResponse, VideoMutationRow, VideoResponse, VideoScopeRow, derivative_object_key,
    digest_credential, digest_identifier, parse_sha256, profile_kind, request_digest,
    source_object_key,
};
use contracts::{
    API_SCHEMA_VERSION, AuthorityResponse, CapabilitiesResponse, CreateVideoRequest,
    DiscoveryResponse, MAX_COMMAND_BODY_BYTES, MAX_SAFE_INTEGER, MediaJobRequest,
    UpdatePrivacyRequest, UploadIntentRequest, normalize_cf_ray, origin_allowed,
    sanitized_public_title, valid_content_type, valid_idempotency_key, valid_uuid,
};
use frame_client::{
    ApiError, ApiVersion, Capabilities, CaptionTrack, Health, PlaybackDescriptor,
    PublicShareSummary, RetryAdvice, ServiceStatus, ShareAvailability,
};
use routing::{
    Deployment, HostPolicy, Route, classify_raw_path, parse_raw_request_target, validate_host,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::*;

const PRODUCTION_HOST: &str = "frame.engmanager.xyz";
#[derive(Debug, Deserialize)]
struct ReadyRow {
    ready: i32,
}

#[derive(Debug, Serialize)]
struct HealthDependencies {
    d1: bool,
    r2: bool,
    media_transformations: bool,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    #[serde(flatten)]
    contract: Health,
    dependencies: HealthDependencies,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicShareRow {
    id: String,
    title: String,
    state: String,
    privacy: String,
    organization_id: Option<String>,
    playback_object_key: Option<String>,
    duration_ms: Option<i64>,
    content_type: Option<String>,
    bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicObject {
    key: String,
    content_type: String,
    bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestedRange {
    range: worker::Range,
    start: u64,
    length: u64,
}

#[derive(Debug, Deserialize)]
struct AuthorityRow {
    phase: String,
    authority: String,
    epoch: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MutationAuthorityFence {
    /// Local deployments deliberately bypass the cutover table. Production
    /// epochs are always non-negative, so -1 is an unambiguous SQL sentinel.
    sql_epoch: i64,
}

impl MutationAuthorityFence {
    const LOCAL_SQL_EPOCH: i64 = -1;

    const fn local() -> Self {
        Self {
            sql_epoch: Self::LOCAL_SQL_EPOCH,
        }
    }

    const fn production(epoch: i64) -> Self {
        Self { sql_epoch: epoch }
    }
}

#[derive(Debug)]
struct AuthenticatedActor {
    user_id: String,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequiredAccess {
    Read,
    Write,
    Admin,
}

impl AuthenticatedActor {
    fn allows(&self, required: RequiredAccess) -> bool {
        self.scopes.iter().any(|scope| {
            scope == "frame:admin"
                || (scope == "frame:write"
                    && matches!(required, RequiredAccess::Read | RequiredAccess::Write))
                || (scope == "frame:read" && required == RequiredAccess::Read)
        })
    }
}

enum CommandReplay {
    New,
    Stored { status: u16, json: String },
    Conflict,
}

struct FakePreview<'a> {
    tenant_id: &'a str,
    video_id: &'a str,
    job_id: &'a str,
    output_key: &'a str,
    source_version: u32,
    source: &'a SourceObjectRow,
}

#[derive(Debug)]
struct RuntimeConfig {
    host_policy: HostPolicy,
    media_mode: MediaMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaMode {
    Remote,
    Fake,
}

impl RuntimeConfig {
    fn from_env(env: &Env) -> Option<Self> {
        let deployment = env
            .var("FRAME_DEPLOYMENT")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "production".into());
        let deployment = match deployment.as_str() {
            "production" | "staging" => Deployment::Production,
            "local" | "development" | "test" => Deployment::Local,
            _ => return None,
        };
        let default_host = if deployment == Deployment::Local {
            "localhost"
        } else {
            PRODUCTION_HOST
        };
        let public_host = env
            .var("FRAME_PUBLIC_HOST")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| default_host.into());
        let media_mode = env
            .var("FRAME_MEDIA_MODE")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "remote".into());
        let media_mode = match (deployment, media_mode.as_str()) {
            (Deployment::Production, "remote") => MediaMode::Remote,
            (Deployment::Local, "fake") => MediaMode::Fake,
            (Deployment::Local, "remote") => MediaMode::Remote,
            _ => return None,
        };
        Some(Self {
            host_policy: HostPolicy::new(deployment, public_host)?,
            media_mode,
        })
    }

    fn production(&self) -> bool {
        self.host_policy.deployment == Deployment::Production
    }
}

#[derive(Debug, Clone, Copy)]
struct ApiFailure {
    status: u16,
    code: &'static str,
    message: &'static str,
    retryable: bool,
    allow: Option<&'static str>,
    authenticate: bool,
}

impl ApiFailure {
    const fn new(status: u16, code: &'static str, message: &'static str, retryable: bool) -> Self {
        Self {
            status,
            code,
            message,
            retryable,
            allow: None,
            authenticate: false,
        }
    }

    const fn with_allow(mut self, allow: &'static str) -> Self {
        self.allow = Some(allow);
        self
    }

    const fn with_authenticate(mut self) -> Self {
        self.authenticate = true;
        self
    }
}

#[event(fetch)]
pub async fn main(request: Request, env: Env, _context: Context) -> Result<Response> {
    let request_id = request_id(&request);
    let Some(config) = RuntimeConfig::from_env(&env) else {
        return failure_response(
            ApiFailure::new(
                503,
                "service_unavailable",
                "The service is temporarily unavailable.",
                true,
            ),
            &request_id,
            true,
        );
    };

    match dispatch(request, &env, &config, &request_id).await {
        Ok(response) => Ok(response),
        Err(_) => {
            console_error!("control-plane request failed request_id={request_id}");
            failure_response(
                ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
                &request_id,
                config.production(),
            )
        }
    }
}

async fn dispatch(
    mut request: Request,
    env: &Env,
    config: &RuntimeConfig,
    request_id: &str,
) -> Result<Response> {
    let target = match parse_raw_request_target(&request.inner().url()) {
        Ok(target) => target,
        Err(_) => {
            return failure_response(
                ApiFailure::new(
                    400,
                    "invalid_request_target",
                    "The request target is invalid.",
                    false,
                ),
                request_id,
                config.production(),
            );
        }
    };
    let host = request.headers().get("host")?;
    if validate_host(&target, host.as_deref(), &config.host_policy).is_err() {
        return failure_response(
            ApiFailure::new(
                421,
                "unexpected_host",
                "The request host is not served here.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let canonical_origin = format!("{}://{}", target.scheme, target.authority);
    let response = match classify_raw_path(&target.path) {
        Route::LegacyRoot => method_guard(&request, &[Method::Get], "GET")?.map_or_else(
            || Response::ok("Frame control plane. See /health."),
            |failure| failure_response(failure, request_id, config.production()),
        )?,
        Route::LegacyHealth | Route::ApiHealth => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                health_response(env, config).await?
            }
        }
        Route::Discovery => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                Response::from_json(&DiscoveryResponse::default())?
            }
        }
        Route::Capabilities => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let capabilities = CapabilitiesResponse {
                    media_jobs: match config.media_mode {
                        MediaMode::Fake => "authenticated_local_fake_preview",
                        MediaMode::Remote => "fail_closed_pending_provider_consumer",
                    },
                    ..CapabilitiesResponse::default()
                };
                Response::from_json(&capabilities)?
            }
        }
        Route::PublicShare { share_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                public_share_response(env, &share_id, &canonical_origin).await?
            }
        }
        Route::PublicMedia { share_id } => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Head], "GET, HEAD")?
            {
                failure_response(failure, request_id, config.production())?
            } else {
                public_media_response(
                    env,
                    &request,
                    &share_id,
                    request.method() == Method::Head,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::VideoCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CreateVideoRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                video_create_response(env, config, &request, &actor, body, request_id).await?
            }
        }
        Route::VideoPrivacy { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Patch], "PATCH")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&video_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<UpdatePrivacyRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                video_privacy_response(env, config, &request, &actor, &video_id, body, request_id)
                    .await?
            }
        }
        Route::UploadIntent => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<UploadIntentRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                upload_intent_response(env, config, &request, &actor, body, request_id).await?
            }
        }
        Route::UploadStatus { upload_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Read,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                upload_status_response(
                    env,
                    &request,
                    &actor,
                    &upload_id,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::UploadContent { upload_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                upload_content_response(env, config, &mut request, &actor, &upload_id, request_id)
                    .await?
            }
        }
        Route::MediaJobCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<MediaJobRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                media_job_create_response(env, config, &request, &actor, body, request_id).await?
            }
        }
        Route::MediaJobStatus { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(
                    invalid_identifier_failure(),
                    request_id,
                    config.production(),
                )?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Read,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                media_job_status_response(
                    env,
                    &request,
                    &actor,
                    &job_id,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::MediaJobCancel { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(
                    invalid_identifier_failure(),
                    request_id,
                    config.production(),
                )?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_idempotency_header(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                media_job_cancel_response(env, config, &request, &actor, &job_id, request_id)
                    .await?
            }
        }
        Route::AuthorityStatus => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                if let Err(failure) =
                    authenticated_command_preflight(&request, env, config, RequiredAccess::Admin)
                        .await?
                {
                    return failure_response(failure, request_id, config.production());
                }
                authority_response(env).await?
            }
        }
        Route::InvalidApiPath => failure_response(
            ApiFailure::new(400, "invalid_api_path", "The API path is invalid.", false),
            request_id,
            config.production(),
        )?,
        Route::UnknownApi => {
            failure_response(not_found_failure(), request_id, config.production())?
        }
        Route::NotApi => failure_response(
            ApiFailure::new(
                404,
                "not_api_route",
                "The requested route is not handled by this service.",
                false,
            ),
            request_id,
            config.production(),
        )?,
    };
    secure_response(response, request_id, config.production())
}

fn method_guard(
    request: &Request,
    accepted: &[Method],
    allow: &'static str,
) -> Result<Option<ApiFailure>> {
    let method = request.method();
    Ok((!accepted.contains(&method)).then(|| {
        ApiFailure::new(
            405,
            "method_not_allowed",
            "The request method is not allowed for this route.",
            false,
        )
        .with_allow(allow)
    }))
}

async fn health_response(env: &Env, config: &RuntimeConfig) -> Result<Response> {
    let database = env.d1("DB")?;
    let ready = database
        .prepare("SELECT 1 AS ready")
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1);
    let _recordings = env.bucket("RECORDINGS")?;
    // Merely having a provider binding is not an executable consumer contract.
    // Remote readiness stays red until the callback/lease implementation exists.
    let media_transformations = config.media_mode == MediaMode::Fake;

    let status = if ready && media_transformations {
        ServiceStatus::Ok
    } else {
        ServiceStatus::Degraded
    };
    Response::from_json(&HealthResponse {
        contract: health_contract(status)?,
        dependencies: HealthDependencies {
            d1: ready,
            r2: true,
            media_transformations,
        },
    })
}

fn health_contract(status: ServiceStatus) -> Result<Health> {
    let contract = Health {
        api_version: ApiVersion::current(),
        service: "frame".into(),
        status,
        release: env!("CARGO_PKG_VERSION").into(),
        capabilities: Capabilities::from_names(vec![
            "public_share_summary".into(),
            "range_playback".into(),
        ])
        .map_err(|_| Error::RustError("public capabilities are invalid".into()))?,
    };
    contract
        .validate()
        .map_err(|_| Error::RustError("health contract is invalid".into()))?;
    Ok(contract)
}

async fn video_create_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: CreateVideoRequest,
    request_id: &str,
) -> Result<Response> {
    let Some(authority_fence) = mutation_authority_fence(env, config).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("video_create", &body)
        .map_err(|()| Error::RustError("video command could not be digested".into()))?;
    match command_replay(
        &database,
        &tenant_id,
        &idempotency_key,
        "video_create",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }

    let video_id = new_id();
    let response = VideoResponse::new(video_id.clone());
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("video response could not be serialized".into()))?;
    let now = current_time_ms()?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "video_id": video_id,
        "state": "pending",
        "privacy": "private",
    })
    .to_string();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO videos(\
                   id, owner_id, title, state, source_object_key, playback_object_key, duration_ms, \
                   created_at_ms, updated_at_ms, organization_id, privacy, metadata_json, revision\
                 ) SELECT ?1, ?2, ?3, 'pending', NULL, NULL, NULL, ?4, ?4, ?5, \
                          'private', '{}', 0 \
                   FROM organization_members m \
                   JOIN organizations o ON o.id = m.organization_id \
                   WHERE m.organization_id = ?5 AND m.user_id = ?2 \
                     AND m.state = 'active' AND m.role IN ('owner', 'admin', 'member') \
                     AND o.status = 'active' \
                     AND (?6 = -1 OR EXISTS (SELECT 1 FROM authority_state a \
                       WHERE a.singleton = 1 AND a.epoch = ?6 AND a.authority = 'd1' \
                         AND a.phase IN ('d1_authoritative', 'finalized')))",
            )
            .bind(&[
                JsValue::from_str(&video_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&body.title),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) SELECT ?1, ?2, 'video_create', ?3, 201, ?4, ?5, ?6 \
                   WHERE EXISTS (SELECT 1 FROM videos v \
                     WHERE v.id = ?7 AND v.organization_id = ?1 AND v.owner_id = ?8 \
                       AND v.deleted_at_ms IS NULL)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
                JsValue::from_str(&video_id),
                JsValue::from_str(&actor.user_id),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) SELECT ?1, ?2, 'video', ?3, 'video.created', ?4, ?5, \
                          'pending', 0, ?6, ?6 FROM videos v \
                   WHERE v.id = ?3 AND v.organization_id = ?2 \
                     AND v.owner_id = ?7 AND v.deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&video_id),
                JsValue::from_str(&format!("video-created:{video_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&actor.user_id),
            ])?,
    ];
    if !atomic_batch_applied(database.batch(statements).await?)? {
        if authorized_tenant(&database, request, actor, RequiredAccess::Write)
            .await?
            .as_deref()
            != Some(tenant_id.as_str())
        {
            return failure_response(not_found_failure(), request_id, config.production());
        }
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    }
    json_response(&response, 201, None)
}

async fn video_privacy_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    video_id: &str,
    body: UpdatePrivacyRequest,
    request_id: &str,
) -> Result<Response> {
    let Some(authority_fence) = mutation_authority_fence(env, config).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("video_privacy", &(video_id, &body))
        .map_err(|()| Error::RustError("privacy command could not be digested".into()))?;
    match command_replay(
        &database,
        &tenant_id,
        &idempotency_key,
        "video_privacy",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(existing) =
        load_video_mutation(&database, &tenant_id, video_id, &actor.user_id).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !existing.actor_can_update(&actor.user_id) {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let expected_revision = i64::try_from(body.expected_revision)
        .map_err(|_| Error::RustError("privacy revision is invalid".into()))?;
    if existing.revision != expected_revision {
        return failure_response(revision_conflict_failure(), request_id, config.production());
    }
    let Some(next_revision) = existing
        .revision
        .checked_add(1)
        .filter(|revision| *revision <= i64::try_from(MAX_SAFE_INTEGER).unwrap_or(i64::MAX))
    else {
        return failure_response(revision_conflict_failure(), request_id, config.production());
    };
    if body.privacy == "public"
        && (existing.state != "ready"
            || !video_has_shareable_media(&database, &tenant_id, video_id).await?)
    {
        return failure_response(
            ApiFailure::new(
                409,
                "video_not_shareable",
                "The video is not ready to be shared.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let mut updated = existing.clone();
    updated.privacy.clone_from(&body.privacy);
    updated.revision = next_revision;
    let response = updated
        .public_response()
        .ok_or_else(|| Error::RustError("privacy response is invalid".into()))?;
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("privacy response could not be serialized".into()))?;
    let now = current_time_ms()?;
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "video_id": video_id,
        "privacy": body.privacy,
        "revision": response.revision,
    })
    .to_string();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) SELECT ?1, ?2, 'video_privacy', ?3, 200, ?4, ?5, ?6 \
                   FROM videos v \
                   JOIN organizations o ON o.id = v.organization_id AND o.status = 'active' \
                   JOIN organization_members m ON m.organization_id = v.organization_id \
                     AND m.user_id = ?8 AND m.state = 'active' \
                   WHERE v.id = ?7 AND v.organization_id = ?1 \
                     AND v.deleted_at_ms IS NULL AND v.revision = ?9 \
                     AND (m.role IN ('owner', 'admin') OR (m.role = 'member' AND (\
                       v.owner_id = ?8 OR EXISTS (SELECT 1 FROM space_videos sv \
                         JOIN spaces s ON s.id = sv.space_id \
                           AND s.organization_id = v.organization_id AND s.deleted_at_ms IS NULL \
                         JOIN space_members sm ON sm.space_id = s.id \
                         WHERE sv.video_id = v.id AND sm.user_id = ?8 AND sm.role = 'manager')))) \
                     AND (?10 = 'private' OR (v.state = 'ready' AND EXISTS (\
                       SELECT 1 FROM object_manifests om \
                       WHERE om.object_key = v.playback_object_key AND om.video_id = v.id \
                         AND om.organization_id = v.organization_id AND om.role = 'preview' \
                         AND om.object_version > 0 AND om.state = 'available' \
                         AND om.bytes BETWEEN 1 AND 9007199254740991 \
                         AND om.content_type LIKE 'video/%' \
                         AND length(om.checksum_sha256) = 64 \
                         AND lower(om.checksum_sha256) = om.checksum_sha256 \
                         AND om.checksum_sha256 NOT GLOB '*[^0-9a-f]*' \
                         AND om.provider_etag IS NOT NULL AND om.provider_etag <> '' \
                         AND substr(om.object_key, 1, length('tenants/' || v.organization_id || \
                           '/videos/' || v.id || '/derivatives/')) = \
                           'tenants/' || v.organization_id || '/videos/' || v.id || '/derivatives/' \
                         AND instr(om.object_key, '..') = 0 \
                         AND instr(om.object_key, char(92)) = 0 \
                         AND instr(om.object_key, '?') = 0 \
                         AND instr(om.object_key, '#') = 0 \
                         AND instr(om.object_key, '%') = 0))) \
                     AND (?11 = -1 OR EXISTS (SELECT 1 FROM authority_state a \
                       WHERE a.singleton = 1 AND a.epoch = ?11 AND a.authority = 'd1' \
                         AND a.phase IN ('d1_authoritative', 'finalized')))",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
                JsValue::from_str(video_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_f64(expected_revision as f64),
                JsValue::from_str(&body.privacy),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET privacy = ?3, updated_at_ms = ?5, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND revision = ?4 \
                   AND deleted_at_ms IS NULL AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?6 \
                       AND c.command_type = 'video_privacy' AND c.request_digest = ?7 \
                       AND c.response_status = 200 AND c.response_json = ?8)",
            )
            .bind(&[
                JsValue::from_str(video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&body.privacy),
                JsValue::from_f64(expected_revision as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) SELECT ?1, ?2, 'video', ?3, 'video.privacy.changed', ?4, ?5, \
                          'pending', 0, ?6, ?6 FROM videos v \
                   JOIN command_idempotency c ON c.organization_id = v.organization_id \
                     AND c.idempotency_key = ?7 AND c.command_type = 'video_privacy' \
                     AND c.request_digest = ?8 AND c.response_json = ?11 \
                   WHERE v.id = ?3 AND v.organization_id = ?2 \
                     AND v.revision = ?9 AND v.privacy = ?10 AND v.deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(video_id),
                JsValue::from_str(&format!("video-privacy:{video_id}:{}", response.revision)),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&body.privacy),
                JsValue::from_str(&response_json),
            ])?,
    ];
    if !atomic_batch_applied(database.batch(statements).await?)? {
        let current_fence = mutation_authority_fence(env, config).await?;
        if current_fence != Some(authority_fence) {
            return failure_response(mutation_disabled_failure(), request_id, config.production());
        }
        let Some(current) =
            load_video_mutation(&database, &tenant_id, video_id, &actor.user_id).await?
        else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        if !current.actor_can_update(&actor.user_id) {
            return failure_response(not_found_failure(), request_id, config.production());
        }
        if current.revision != expected_revision {
            return failure_response(revision_conflict_failure(), request_id, config.production());
        }
        if body.privacy == "public"
            && (current.state != "ready"
                || !video_has_shareable_media(&database, &tenant_id, video_id).await?)
        {
            return failure_response(
                ApiFailure::new(
                    409,
                    "video_not_shareable",
                    "The video is not ready to be shared.",
                    true,
                ),
                request_id,
                config.production(),
            );
        }
        return Err(Error::RustError(
            "privacy command made no progress despite valid fences".into(),
        ));
    }
    json_response(&response, 200, response.public_share_path.as_deref())
}

async fn upload_intent_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: UploadIntentRequest,
    request_id: &str,
) -> Result<Response> {
    if !mutation_authority_enabled(env, config).await? {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    if body.role != "source" {
        return failure_response(
            invalid_body_failure("unsupported_object_role"),
            request_id,
            config.production(),
        );
    }
    if !supported_source_content_type(&body.content_type) {
        return failure_response(
            invalid_body_failure("unsupported_media_type"),
            request_id,
            config.production(),
        );
    }
    if body.expected_bytes > MAX_SINGLE_UPLOAD_BYTES {
        return failure_response(
            ApiFailure::new(
                413,
                "multipart_required",
                "This upload requires the multipart transport.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("upload_intent", &body)
        .map_err(|()| Error::RustError("upload command could not be digested".into()))?;
    match command_replay(
        &database,
        &tenant_id,
        &idempotency_key,
        "upload_intent",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }

    let Some(integration) = active_r2_integration(&database, &tenant_id).await? else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    if !integration.supports_single_put() {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    }

    if !video_is_scoped(&database, &tenant_id, &body.video_id).await? {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let upload_id = new_id();
    let resource_idempotency_key = digest_identifier(
        "upload_resource",
        &format!("{tenant_id}:{idempotency_key}:{upload_id}"),
    )
    .map_err(|()| Error::RustError("upload resource identity is invalid".into()))?;
    let object_key = source_object_key(&tenant_id, &body.video_id, &body.role, body.object_version);
    let response = UploadIntentResponse::new(
        upload_id.clone(),
        body.expected_bytes,
        body.content_type.clone(),
    );
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("upload response could not be serialized".into()))?;
    let now = current_time_ms()?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "upload_id": upload_id,
        "video_id": body.video_id,
        "role": body.role,
        "object_version": body.object_version,
    })
    .to_string();

    let statements = vec![
        database
            .prepare(
                "INSERT INTO video_uploads(\
                   id, organization_id, video_id, state, expected_bytes, received_bytes, \
                   idempotency_key, source_object_key, source_version, content_type, \
                   created_at_ms, updated_at_ms, revision\
                 ) VALUES (?1, ?2, ?3, 'initiated', ?4, 0, ?5, ?6, ?7, ?8, ?9, ?9, 0)",
            )
            .bind(&[
                JsValue::from_str(&upload_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&body.video_id),
                JsValue::from_f64(body.expected_bytes as f64),
                JsValue::from_str(&resource_idempotency_key),
                JsValue::from_str(&object_key),
                JsValue::from_f64(f64::from(body.object_version)),
                JsValue::from_str(&body.content_type),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET state = 'uploading', updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&body.video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) VALUES (?1, ?2, 'upload_intent', ?3, 201, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) VALUES (?1, ?2, 'video_upload', ?3, 'upload.intent.created', ?4, ?5, \
                           'pending', 0, ?6, ?6)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload_id),
                JsValue::from_str(&format!("upload-intent:{upload_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
            ])?,
    ];
    require_batch_success(database.batch(statements).await?)?;
    json_response(&response, 201, Some(&response.upload_path))
}

async fn upload_status_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    upload_id: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Read).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(upload) = load_upload(&database, &tenant_id, upload_id).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let status = upload
        .public_status()
        .ok_or_else(|| Error::RustError("upload state is invalid".into()))?;
    json_response(&status, 200, None)
}

async fn upload_content_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &mut Request,
    actor: &AuthenticatedActor,
    upload_id: &str,
    request_id: &str,
) -> Result<Response> {
    if !mutation_authority_enabled(env, config).await? {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(upload) = load_upload(&database, &tenant_id, upload_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if upload.organization_id != tenant_id || upload.id != upload_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }

    if !matches!(
        upload.state.as_str(),
        "initiated" | "uploading" | "finalizing" | "complete"
    ) {
        return failure_response(
            ApiFailure::new(
                409,
                "upload_not_writable",
                "The upload is not writable in its current state.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let expected_bytes = u64::try_from(upload.expected_bytes)
        .ok()
        .filter(|bytes| *bytes > 0 && *bytes <= MAX_SINGLE_UPLOAD_BYTES)
        .ok_or_else(|| Error::RustError("upload byte contract is invalid".into()))?;
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<u64>().ok());
    if content_length != Some(expected_bytes) {
        return failure_response(
            invalid_body_failure("content_length_mismatch"),
            request_id,
            config.production(),
        );
    }
    if request.headers().get("content-type")?.as_deref() != Some(&upload.content_type) {
        return failure_response(
            invalid_body_failure("content_type_mismatch"),
            request_id,
            config.production(),
        );
    }
    if request
        .headers()
        .get("content-encoding")?
        .is_some_and(|encoding| encoding != "identity")
    {
        return failure_response(
            invalid_body_failure("unsupported_content_encoding"),
            request_id,
            config.production(),
        );
    }
    let checksum_text = request
        .headers()
        .get("x-content-sha256")?
        .filter(|value| value.bytes().all(|byte| !byte.is_ascii_uppercase()));
    let Some((checksum_text, checksum)) = checksum_text
        .as_deref()
        .and_then(|value| parse_sha256(value).map(|checksum| (value.to_owned(), checksum)))
    else {
        return failure_response(
            invalid_body_failure("invalid_content_checksum"),
            request_id,
            config.production(),
        );
    };
    if upload.state == "complete" {
        if upload.checksum_sha256.as_deref() != Some(checksum_text.as_str()) {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        if completed_upload_matches(env, &upload).await? {
            let status = upload
                .public_status()
                .ok_or_else(|| Error::RustError("upload state is invalid".into()))?;
            return json_response(&status, 200, None);
        }
        return failure_response(
            media_unavailable_failure("upload_reconciliation_required"),
            request_id,
            config.production(),
        );
    }
    let integration = active_r2_integration(&database, &tenant_id).await?;
    let Some(integration) = integration else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    if !integration.supports_single_put() {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let now = current_time_ms()?;
    database
        .prepare(
            "UPDATE video_uploads SET state = 'finalizing', updated_at_ms = ?3, revision = revision + 1 \
             WHERE id = ?1 AND organization_id = ?2 \
               AND state IN ('initiated', 'uploading', 'finalizing')",
        )
        .bind(&[
            JsValue::from_str(upload_id),
            JsValue::from_str(&tenant_id),
            JsValue::from_f64(now as f64),
        ])?
        .run()
        .await?;

    let bucket = env.bucket("RECORDINGS")?;
    let existing = bucket.head(&upload.source_object_key).await?;
    let object = if let Some(existing) = existing {
        existing
    } else {
        let stream = FixedLengthStream::wrap(request.stream()?, expected_bytes);
        let metadata = HttpMetadata {
            content_type: Some(upload.content_type.clone()),
            content_disposition: Some("attachment".into()),
            cache_control: Some("private, no-store".into()),
            ..HttpMetadata::default()
        };
        bucket
            .put(&upload.source_object_key, stream)
            .http_metadata(metadata)
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
            .ok_or_else(|| Error::RustError("conditional upload was not applied".into()))?
    };
    let metadata = object.http_metadata();
    if object.size() != expected_bytes
        || object.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || metadata.content_type.as_deref() != Some(upload.content_type.as_str())
        || metadata.content_encoding.is_some()
    {
        return failure_response(
            media_unavailable_failure("upload_checksum_mismatch"),
            request_id,
            config.production(),
        );
    }

    let etag = object.etag();
    let storage_object_id = new_id();
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "upload_id": upload.id,
        "video_id": upload.video_id,
        "source_version": upload.source_version,
        "bytes": expected_bytes,
    })
    .to_string();
    let statements = vec![
        database
            .prepare(
                "UPDATE video_uploads \
                 SET state = 'complete', received_bytes = expected_bytes, checksum_sha256 = ?3, \
                     updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND state = 'finalizing'",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&checksum_text),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO object_manifests(\
                   object_key, video_id, role, bytes, checksum_sha256, content_type, created_at_ms, \
                   organization_id, object_version, provider_etag, state, updated_at_ms\
                 ) VALUES (?1, ?2, 'source', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'available', ?6) \
                 ON CONFLICT(object_key) DO UPDATE SET \
                   bytes = excluded.bytes, checksum_sha256 = excluded.checksum_sha256, \
                   content_type = excluded.content_type, provider_etag = excluded.provider_etag, \
                   state = 'available', updated_at_ms = excluded.updated_at_ms \
                 WHERE object_manifests.video_id = excluded.video_id \
                   AND object_manifests.organization_id = excluded.organization_id \
                   AND object_manifests.role = excluded.role \
                   AND object_manifests.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_str(&upload.video_id),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&checksum_text),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&etag),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(\
                   id, organization_id, integration_id, video_id, object_key, role, object_version, \
                   state, bytes, content_type, checksum_sha256, provider_etag, created_at_ms\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'source', ?6, 'available', ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT(integration_id, object_key) DO UPDATE SET \
                   state = 'available', bytes = excluded.bytes, content_type = excluded.content_type, \
                   checksum_sha256 = excluded.checksum_sha256, provider_etag = excluded.provider_etag \
                 WHERE storage_objects.organization_id = excluded.organization_id \
                   AND storage_objects.video_id = excluded.video_id \
                   AND storage_objects.role = excluded.role \
                   AND storage_objects.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_str(&checksum_text),
                JsValue::from_str(&etag),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET source_object_key = ?3, state = 'processing', \
                    updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) VALUES (?1, ?2, 'video_upload', ?3, 'upload.completed', ?4, ?5, \
                           'pending', 0, ?6, ?6)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(upload_id),
                JsValue::from_str(&format!("upload-complete:{upload_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
            ])?,
    ];
    require_batch_success(database.batch(statements).await?)?;
    let status = UploadStatusResponse {
        schema_version: API_SCHEMA_VERSION,
        upload_id: upload_id.into(),
        state: "complete".into(),
        expected_bytes,
        received_bytes: expected_bytes,
        content_type: upload.content_type,
    };
    json_response(&status, 200, None)
}

async fn media_job_create_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: MediaJobRequest,
    request_id: &str,
) -> Result<Response> {
    if !mutation_authority_enabled(env, config).await? {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    }
    if config.media_mode == MediaMode::Remote {
        return failure_response(
            ApiFailure::new(
                503,
                "media_executor_unavailable",
                "The media executor is temporarily unavailable.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("media_job_create", &body)
        .map_err(|()| Error::RustError("media command could not be digested".into()))?;
    match command_replay(
        &database,
        &tenant_id,
        &idempotency_key,
        "media_job_create",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    if !video_is_scoped(&database, &tenant_id, &body.video_id).await? {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(source) =
        load_source_object(&database, &tenant_id, &body.video_id, body.source_version).await?
    else {
        return failure_response(
            ApiFailure::new(
                409,
                "source_not_ready",
                "The source object is not ready for processing.",
                true,
            ),
            request_id,
            config.production(),
        );
    };
    if !supported_source_content_type(&source.content_type) {
        return failure_response(
            invalid_body_failure("unsupported_source_media_type"),
            request_id,
            config.production(),
        );
    }
    if config.media_mode == MediaMode::Fake && body.profile != "preview_v1" {
        return failure_response(
            ApiFailure::new(
                422,
                "profile_unavailable",
                "The selected media profile is unavailable in this runtime.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let kind = profile_kind(&body.profile)
        .ok_or_else(|| Error::RustError("validated profile is unsupported".into()))?;
    let executor = match config.media_mode {
        MediaMode::Remote => "cloudflare_media",
        MediaMode::Fake => "native_gstreamer",
    };
    let job_id = new_id();
    let output_key = derivative_object_key(
        &tenant_id,
        &body.video_id,
        &body.profile,
        body.source_version,
    );
    let response = MediaJobResponse::new(job_id.clone(), body.profile.clone(), executor.into());
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("media response could not be serialized".into()))?;
    let payload_json = serde_json::to_string(&body)
        .map_err(|_| Error::RustError("media request could not be serialized".into()))?;
    let now = current_time_ms()?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "video_id": body.video_id,
        "profile": body.profile,
        "source_version": body.source_version,
    })
    .to_string();
    let scoped_job_idempotency_key = digest_identifier(
        "media_job_resource",
        &format!("{tenant_id}:{idempotency_key}:{job_id}"),
    )
    .map_err(|()| Error::RustError("media job resource identity is invalid".into()))?;
    let statements = vec![
        database
            .prepare(
                "INSERT INTO media_jobs(\
                   id, video_id, kind, state, idempotency_key, attempt, payload_json, \
                   created_at_ms, updated_at_ms, organization_id, selected_executor, \
                   source_version, profile_version, output_object_key, cancel_requested, revision\
                 ) VALUES (?1, ?2, ?3, 'queued', ?4, 0, ?5, ?6, ?6, ?7, ?8, ?9, 1, ?10, 0, 0)",
            )
            .bind(&[
                JsValue::from_str(&job_id),
                JsValue::from_str(&body.video_id),
                JsValue::from_str(kind),
                JsValue::from_str(&scoped_job_idempotency_key),
                JsValue::from_str(&payload_json),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(executor),
                JsValue::from_f64(f64::from(body.source_version)),
                JsValue::from_str(&output_key),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) VALUES (?1, ?2, 'media_job_create', ?3, 202, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) VALUES (?1, ?2, 'media_job', ?3, 'media.job.queued', ?4, ?5, \
                           'pending', 0, ?6, ?6)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&job_id),
                JsValue::from_str(&format!("media-job:{job_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
            ])?,
    ];
    require_batch_success(database.batch(statements).await?)?;

    let mut response = response;
    if config.media_mode == MediaMode::Fake && body.profile == "preview_v1" {
        if complete_fake_preview(
            env,
            &database,
            FakePreview {
                tenant_id: &tenant_id,
                video_id: &body.video_id,
                job_id: &job_id,
                output_key: &output_key,
                source_version: body.source_version,
                source: &source,
            },
        )
        .await
        .is_err()
        {
            mark_fake_job_failed(&database, &tenant_id, &job_id).await?;
        }
        let current = load_media_job(&database, &tenant_id, &job_id)
            .await?
            .ok_or_else(|| Error::RustError("created media job disappeared".into()))?;
        response.state = current.state;
        let response_json = serde_json::to_string(&response)
            .map_err(|_| Error::RustError("media response could not be serialized".into()))?;
        database
            .prepare(
                "UPDATE command_idempotency SET response_json = ?4 \
                 WHERE organization_id = ?1 AND idempotency_key = ?2 \
                   AND command_type = 'media_job_create' AND request_digest = ?3",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
            ])?
            .run()
            .await?;
    }
    json_response(&response, 202, Some(&response.status_path))
}

async fn media_job_status_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Read).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(job) = load_media_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let status = job
        .public_status()
        .ok_or_else(|| Error::RustError("media job state is invalid".into()))?;
    json_response(&status, 200, None)
}

async fn media_job_cancel_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    request_id: &str,
) -> Result<Response> {
    if !mutation_authority_enabled(env, config).await? {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let digest = digest_identifier("media_job_cancel", job_id)
        .map_err(|()| Error::RustError("cancel command could not be digested".into()))?;
    match command_replay(
        &database,
        &tenant_id,
        &idempotency_key,
        "media_job_cancel",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(existing) = load_media_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let now = current_time_ms()?;
    if matches!(existing.state.as_str(), "succeeded" | "failed") {
        return failure_response(
            ApiFailure::new(
                409,
                "job_terminal",
                "A terminal media job cannot be cancelled.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let updated = database
        .prepare(
            "UPDATE media_jobs SET cancel_requested = 1, \
               state = CASE WHEN state = 'queued' THEN 'cancelled' ELSE state END, \
               progress_basis_points = CASE WHEN state = 'queued' THEN 0 ELSE progress_basis_points END, \
               updated_at_ms = ?3, revision = revision + 1 \
             WHERE id = ?1 AND organization_id = ?2 \
               AND state IN ('queued', 'leased', 'running') AND cancel_requested = 0 \
             RETURNING id, state, json_extract(payload_json, '$.profile') AS profile, \
               selected_executor, progress_basis_points, attempt, \
               cancel_requested, error_class, created_at_ms, updated_at_ms",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_str(&tenant_id),
            JsValue::from_f64(now as f64),
        ])?
        .first::<MediaJobRow>(None)
        .await?;
    let job = if let Some(job) = updated {
        job
    } else {
        let Some(job) = load_media_job(&database, &tenant_id, job_id).await? else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        if matches!(job.state.as_str(), "succeeded" | "failed") {
            return failure_response(
                ApiFailure::new(
                    409,
                    "job_terminal",
                    "A terminal media job cannot be cancelled.",
                    false,
                ),
                request_id,
                config.production(),
            );
        }
        if job.state != "cancelled" && job.cancel_requested != 1 {
            return failure_response(
                ApiFailure::new(
                    409,
                    "job_state_conflict",
                    "The media job changed while cancellation was requested.",
                    true,
                ),
                request_id,
                config.production(),
            );
        }
        job
    };
    let status = job
        .public_status()
        .ok_or_else(|| Error::RustError("media job state is invalid".into()))?;
    let response_json = serde_json::to_string(&status)
        .map_err(|_| Error::RustError("cancel response could not be serialized".into()))?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "state": status.state,
        "cancel_requested": true,
    })
    .to_string();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) VALUES (?1, ?2, 'media_job_cancel', ?3, 200, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) VALUES (?1, ?2, 'media_job', ?3, 'media.job.cancel_requested', ?4, ?5, \
                           'pending', 0, ?6, ?6) \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-cancel:{job_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
            ])?,
    ];
    require_batch_success(database.batch(statements).await?)?;
    json_response(&status, 200, None)
}

async fn complete_fake_preview(
    env: &Env,
    database: &D1Database,
    command: FakePreview<'_>,
) -> Result<()> {
    let FakePreview {
        tenant_id,
        video_id,
        job_id,
        output_key,
        source_version,
        source,
    } = command;
    let source_bytes = u64::try_from(source.bytes)
        .ok()
        .filter(|value| *value > 0 && *value <= MAX_SINGLE_UPLOAD_BYTES)
        .ok_or_else(|| Error::RustError("fake source size is invalid".into()))?;
    let checksum_text = source
        .checksum_sha256
        .as_deref()
        .ok_or_else(|| Error::RustError("fake source checksum is missing".into()))?;
    let checksum = parse_sha256(checksum_text)
        .ok_or_else(|| Error::RustError("fake source checksum is invalid".into()))?;
    let integration = active_r2_integration(database, tenant_id)
        .await?
        .ok_or_else(|| Error::RustError("fake R2 integration is unavailable".into()))?;
    let bucket = env.bucket("RECORDINGS")?;
    let output = if let Some(output) = bucket.head(output_key).await? {
        output
    } else {
        let source_object = bucket
            .get(&source.object_key)
            .execute()
            .await?
            .filter(|object| object.size() == source_bytes)
            .ok_or_else(|| Error::RustError("fake source object is unavailable".into()))?;
        if source_object.checksum().sha256.as_deref() != Some(checksum.as_slice()) {
            return Err(Error::RustError(
                "fake source object failed checksum verification".into(),
            ));
        }
        let stream = FixedLengthStream::wrap(
            source_object
                .body()
                .ok_or_else(|| Error::RustError("fake source body is unavailable".into()))?
                .stream()?,
            source_bytes,
        );
        bucket
            .put(output_key, stream)
            .http_metadata(HttpMetadata {
                content_type: Some(source.content_type.clone()),
                content_disposition: Some("inline".into()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
            .ok_or_else(|| Error::RustError("fake derivative write conflicted".into()))?
    };
    if output.size() != source_bytes
        || output.checksum().sha256.as_deref() != Some(checksum.as_slice())
    {
        return Err(Error::RustError(
            "fake derivative failed checksum verification".into(),
        ));
    }

    let now = current_time_ms()?;
    let storage_object_id = new_id();
    let attempt_id = new_id();
    let completion_lease = digest_identifier("fake_completion_lease", &attempt_id)
        .map_err(|()| Error::RustError("fake completion lease is invalid".into()))?;
    let claimed = database
        .prepare(
            "UPDATE media_jobs SET state = 'running', worker_id = ?3, lease_token_digest = ?4, \
               heartbeat_at_ms = ?5, updated_at_ms = ?5, revision = revision + 1 \
             WHERE id = ?1 AND organization_id = ?2 AND state = 'queued' \
               AND cancel_requested = 0 RETURNING id",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_str(tenant_id),
            JsValue::from_str(&attempt_id),
            JsValue::from_str(&completion_lease),
            JsValue::from_f64(now as f64),
        ])?
        .first::<VideoScopeRow>(None)
        .await?;
    if claimed.is_none() {
        return Err(Error::RustError(
            "fake media job is no longer eligible for completion".into(),
        ));
    }
    let outbox_id = new_id();
    let output_etag = output.etag();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "video_id": video_id,
        "executor": "local_fake_native_gstreamer",
    })
    .to_string();
    let statements = vec![
        database
            .prepare(
                "UPDATE media_jobs SET state = 'succeeded', attempt = 1, progress_basis_points = 10000, \
                   error_code = NULL, error_class = NULL, updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND state = 'running' \
                   AND cancel_requested = 0 AND lease_token_digest = ?4",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO media_job_attempts(\
                   job_id, attempt, executor, worker_id, started_at_ms, finished_at_ms, outcome\
                 ) SELECT ?1, 1, 'native_gstreamer', ?2, ?3, ?3, 'succeeded' \
                   FROM media_jobs WHERE id = ?1 AND organization_id = ?4 \
                     AND state = 'succeeded' AND lease_token_digest = ?5 \
                 ON CONFLICT(job_id, attempt) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(&attempt_id),
                JsValue::from_f64(now as f64),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO object_manifests(\
                   object_key, video_id, role, bytes, checksum_sha256, content_type, created_at_ms, \
                   organization_id, object_version, provider_etag, state, updated_at_ms\
                 ) SELECT ?1, ?2, 'preview', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'available', ?6 \
                   FROM media_jobs WHERE id = ?10 AND organization_id = ?7 \
                     AND state = 'succeeded' AND lease_token_digest = ?11 \
                 ON CONFLICT(object_key) DO UPDATE SET \
                   bytes = excluded.bytes, checksum_sha256 = excluded.checksum_sha256, \
                   content_type = excluded.content_type, provider_etag = excluded.provider_etag, \
                   state = 'available', updated_at_ms = excluded.updated_at_ms \
                 WHERE object_manifests.video_id = excluded.video_id \
                   AND object_manifests.organization_id = excluded.organization_id \
                   AND object_manifests.role = excluded.role \
                   AND object_manifests.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(output_key),
                JsValue::from_str(video_id),
                JsValue::from_f64(source_bytes as f64),
                JsValue::from_str(checksum_text),
                JsValue::from_str(&source.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(f64::from(source_version)),
                JsValue::from_str(&output_etag),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(\
                   id, organization_id, integration_id, video_id, object_key, role, object_version, \
                   state, bytes, content_type, checksum_sha256, provider_etag, created_at_ms\
                 ) SELECT ?1, ?2, ?3, ?4, ?5, 'preview', ?6, 'available', ?7, ?8, ?9, ?10, ?11 \
                   FROM media_jobs WHERE id = ?12 AND organization_id = ?2 \
                     AND state = 'succeeded' AND lease_token_digest = ?13 \
                 ON CONFLICT(integration_id, object_key) DO UPDATE SET \
                   state = 'available', bytes = excluded.bytes, content_type = excluded.content_type, \
                   checksum_sha256 = excluded.checksum_sha256, provider_etag = excluded.provider_etag \
                 WHERE storage_objects.organization_id = excluded.organization_id \
                   AND storage_objects.video_id = excluded.video_id \
                   AND storage_objects.role = excluded.role \
                   AND storage_objects.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(video_id),
                JsValue::from_str(output_key),
                JsValue::from_f64(f64::from(source_version)),
                JsValue::from_f64(source_bytes as f64),
                JsValue::from_str(&source.content_type),
                JsValue::from_str(checksum_text),
                JsValue::from_str(&output_etag),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "UPDATE videos SET playback_object_key = ?3, state = 'ready', \
                    updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?5 \
                     AND j.organization_id = ?2 AND j.state = 'succeeded' \
                     AND j.lease_token_digest = ?6)",
            )
            .bind(&[
                JsValue::from_str(video_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(output_key),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms\
                 ) SELECT ?1, ?2, 'media_job', ?3, 'media.job.succeeded', ?4, ?5, \
                           'pending', 0, ?6, ?6 FROM media_jobs \
                   WHERE id = ?3 AND organization_id = ?2 AND state = 'succeeded' \
                     AND lease_token_digest = ?7 \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-succeeded:{job_id}")),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&completion_lease),
            ])?,
    ];
    require_batch_success(database.batch(statements).await?)?;
    let completed = load_media_job(database, tenant_id, job_id).await?;
    if !completed.is_some_and(|job| job.state == "succeeded" && job.cancel_requested == 0) {
        return Err(Error::RustError(
            "fake media completion lost its state fence".into(),
        ));
    }
    Ok(())
}

async fn mark_fake_job_failed(database: &D1Database, tenant_id: &str, job_id: &str) -> Result<()> {
    let now = current_time_ms()?;
    database
        .prepare(
            "UPDATE media_jobs SET state = 'failed', attempt = attempt + 1, \
               error_code = 'executor_failure', error_class = 'fake_executor_failure', \
               updated_at_ms = ?3, revision = revision + 1 \
             WHERE id = ?1 AND organization_id = ?2 \
               AND state IN ('queued', 'running') AND cancel_requested = 0",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_str(tenant_id),
            JsValue::from_f64(now as f64),
        ])?
        .run()
        .await?;
    Ok(())
}

async fn command_replay(
    database: &D1Database,
    tenant_id: &str,
    key: &str,
    command_type: &str,
    digest: &str,
) -> Result<CommandReplay> {
    let row = database
        .prepare(
            "SELECT command_type, request_digest, response_status, response_json, expires_at_ms \
             FROM command_idempotency WHERE organization_id = ?1 AND idempotency_key = ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(key)])?
        .first::<StoredCommandRow>(None)
        .await?;
    let Some(row) = row else {
        return Ok(CommandReplay::New);
    };
    if row.expires_at_ms <= current_time_ms()? {
        database
            .prepare(
                "DELETE FROM command_idempotency \
                 WHERE organization_id = ?1 AND idempotency_key = ?2 AND expires_at_ms = ?3",
            )
            .bind(&[
                JsValue::from_str(tenant_id),
                JsValue::from_str(key),
                JsValue::from_f64(row.expires_at_ms as f64),
            ])?
            .run()
            .await?;
        return Ok(CommandReplay::New);
    }
    if row.command_type != command_type || row.request_digest != digest {
        return Ok(CommandReplay::Conflict);
    }
    match (row.response_status, row.response_json) {
        (Some(status), Some(json)) if (200..=299).contains(&status) && json.len() <= 64 * 1_024 => {
            Ok(CommandReplay::Stored {
                status: u16::try_from(status)
                    .map_err(|_| Error::RustError("stored command status is invalid".into()))?,
                json,
            })
        }
        _ => Err(Error::RustError(
            "stored command response is incomplete".into(),
        )),
    }
}

async fn video_is_scoped(database: &D1Database, tenant_id: &str, video_id: &str) -> Result<bool> {
    Ok(database
        .prepare(
            "SELECT id FROM videos \
             WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(video_id), JsValue::from_str(tenant_id)])?
        .first::<VideoScopeRow>(None)
        .await?
        .is_some_and(|row| row.id == video_id))
}

async fn load_video_mutation(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
    actor_id: &str,
) -> Result<Option<VideoMutationRow>> {
    database
        .prepare(
            "SELECT v.id, v.owner_id, v.state, v.privacy, v.revision, \
                    m.role AS actor_role, EXISTS (SELECT 1 FROM space_videos sv \
                      JOIN spaces s ON s.id = sv.space_id \
                        AND s.organization_id = v.organization_id AND s.deleted_at_ms IS NULL \
                      JOIN space_members sm ON sm.space_id = s.id \
                      WHERE sv.video_id = v.id AND sm.user_id = ?3 AND sm.role = 'manager' \
                    ) AS actor_manages_space \
             FROM videos v \
             JOIN organizations o ON o.id = v.organization_id AND o.status = 'active' \
             JOIN organization_members m ON m.organization_id = v.organization_id \
               AND m.user_id = ?3 AND m.state = 'active' \
             WHERE v.id = ?1 AND v.organization_id = ?2 \
               AND v.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(video_id),
            JsValue::from_str(tenant_id),
            JsValue::from_str(actor_id),
        ])?
        .first::<VideoMutationRow>(None)
        .await
}

async fn video_has_shareable_media(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
) -> Result<bool> {
    Ok(database
        .prepare(
            "SELECT 1 AS ready FROM videos v \
             JOIN object_manifests m ON m.object_key = v.playback_object_key \
               AND m.video_id = v.id AND m.organization_id = v.organization_id \
             WHERE v.id = ?1 AND v.organization_id = ?2 AND v.state = 'ready' \
               AND v.deleted_at_ms IS NULL AND m.role = 'preview' \
               AND m.object_version > 0 AND m.state = 'available' \
               AND m.bytes BETWEEN 1 AND 9007199254740991 \
               AND m.content_type LIKE 'video/%' \
               AND length(m.checksum_sha256) = 64 \
               AND lower(m.checksum_sha256) = m.checksum_sha256 \
               AND m.checksum_sha256 NOT GLOB '*[^0-9a-f]*' \
               AND m.provider_etag IS NOT NULL AND m.provider_etag <> '' \
               AND substr(m.object_key, 1, length('tenants/' || v.organization_id || \
                 '/videos/' || v.id || '/derivatives/')) = \
                 'tenants/' || v.organization_id || '/videos/' || v.id || '/derivatives/' \
               AND instr(m.object_key, '..') = 0 \
               AND instr(m.object_key, char(92)) = 0 \
               AND instr(m.object_key, '?') = 0 AND instr(m.object_key, '#') = 0 \
               AND instr(m.object_key, '%') = 0 LIMIT 1",
        )
        .bind(&[JsValue::from_str(video_id), JsValue::from_str(tenant_id)])?
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1))
}

async fn load_upload(
    database: &D1Database,
    tenant_id: &str,
    upload_id: &str,
) -> Result<Option<UploadRow>> {
    database
        .prepare(
            "SELECT id, organization_id, video_id, state, expected_bytes, received_bytes, \
                    source_object_key, source_version, content_type, checksum_sha256 \
             FROM video_uploads WHERE id = ?1 AND organization_id = ?2 LIMIT 1",
        )
        .bind(&[JsValue::from_str(upload_id), JsValue::from_str(tenant_id)])?
        .first::<UploadRow>(None)
        .await
}

async fn load_source_object(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
    source_version: u32,
) -> Result<Option<SourceObjectRow>> {
    database
        .prepare(
            "SELECT object_key, bytes, checksum_sha256, content_type \
             FROM object_manifests \
             WHERE organization_id = ?1 AND video_id = ?2 AND object_version = ?3 \
               AND role IN ('source', 'import') AND state = 'available' \
             ORDER BY CASE role WHEN 'source' THEN 0 ELSE 1 END LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(video_id),
            JsValue::from_f64(f64::from(source_version)),
        ])?
        .first::<SourceObjectRow>(None)
        .await
}

async fn active_r2_integration(
    database: &D1Database,
    tenant_id: &str,
) -> Result<Option<IntegrationRow>> {
    database
        .prepare(
            "SELECT id, capabilities_json FROM storage_integrations \
             WHERE organization_id = ?1 AND provider = 'r2' AND state = 'active' \
             ORDER BY created_at_ms, id LIMIT 1",
        )
        .bind(&[JsValue::from_str(tenant_id)])?
        .first::<IntegrationRow>(None)
        .await
}

async fn load_media_job(
    database: &D1Database,
    tenant_id: &str,
    job_id: &str,
) -> Result<Option<MediaJobRow>> {
    database
        .prepare(
            "SELECT id, state, json_extract(payload_json, '$.profile') AS profile, \
                    selected_executor, progress_basis_points, attempt, \
                    cancel_requested, error_class, created_at_ms, updated_at_ms \
             FROM media_jobs WHERE id = ?1 AND organization_id = ?2 LIMIT 1",
        )
        .bind(&[JsValue::from_str(job_id), JsValue::from_str(tenant_id)])?
        .first::<MediaJobRow>(None)
        .await
}

async fn completed_upload_matches(env: &Env, upload: &UploadRow) -> Result<bool> {
    let Some(expected_checksum) = upload.checksum_sha256.as_deref().and_then(parse_sha256) else {
        return Ok(false);
    };
    let Some(object) = env
        .bucket("RECORDINGS")?
        .head(&upload.source_object_key)
        .await?
    else {
        return Ok(false);
    };
    let metadata = object.http_metadata();
    Ok(
        object.size() == u64::try_from(upload.expected_bytes).unwrap_or(u64::MAX)
            && object.checksum().sha256.as_deref() == Some(expected_checksum.as_slice())
            && metadata.content_type.as_deref() == Some(upload.content_type.as_str())
            && metadata.content_encoding.is_none(),
    )
}

async fn mutation_authority_enabled(env: &Env, config: &RuntimeConfig) -> Result<bool> {
    if !config.production() {
        return Ok(true);
    }
    let row = env
        .d1("DB")?
        .prepare("SELECT phase, authority, epoch FROM authority_state WHERE singleton = 1")
        .first::<AuthorityRow>(None)
        .await?;
    Ok(row.is_some_and(|row| d1_mutation_pair(&row) && row.epoch >= 0))
}

async fn mutation_authority_fence(
    env: &Env,
    config: &RuntimeConfig,
) -> Result<Option<MutationAuthorityFence>> {
    if !config.production() {
        return Ok(Some(MutationAuthorityFence::local()));
    }
    let row = env
        .d1("DB")?
        .prepare("SELECT phase, authority, epoch FROM authority_state WHERE singleton = 1")
        .first::<AuthorityRow>(None)
        .await?;
    Ok(row.and_then(|row| {
        (d1_mutation_pair(&row)
            && (0..=i64::try_from(MAX_SAFE_INTEGER).unwrap_or(i64::MAX)).contains(&row.epoch))
        .then(|| MutationAuthorityFence::production(row.epoch))
    }))
}

fn d1_mutation_pair(row: &AuthorityRow) -> bool {
    matches!(
        (row.phase.as_str(), row.authority.as_str()),
        ("d1_authoritative" | "finalized", "d1")
    )
}

fn tenant_header(request: &Request) -> Result<Option<String>> {
    Ok(request
        .headers()
        .get("x-frame-tenant-id")?
        .filter(|value| valid_uuid(value)))
}

fn supported_source_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "video/mp4" | "video/quicktime" | "video/webm" | "video/x-matroska"
    )
}

fn idempotency_header(request: &Request) -> Result<String> {
    request
        .headers()
        .get("idempotency-key")?
        .filter(|value| valid_idempotency_key(value))
        .ok_or_else(|| Error::RustError("validated idempotency key is unavailable".into()))
}

fn current_time_ms() -> Result<i64> {
    let now = js_sys::Date::now().floor();
    if !now.is_finite() || !(0.0..=MAX_SAFE_INTEGER as f64).contains(&now) {
        return Err(Error::RustError("runtime clock is invalid".into()));
    }
    Ok(now as i64)
}

fn new_id() -> String {
    Uuid::now_v7().to_string()
}

fn require_batch_success(results: Vec<D1Result>) -> Result<()> {
    if results.is_empty() || results.iter().any(|result| !result.success()) {
        return Err(Error::RustError("database command batch failed".into()));
    }
    Ok(())
}

fn classify_atomic_changes(changes: &[usize]) -> std::result::Result<bool, ()> {
    if changes.is_empty() {
        return Err(());
    }
    if changes.iter().all(|changes| *changes == 1) {
        return Ok(true);
    }
    if changes.iter().all(|changes| *changes == 0) {
        return Ok(false);
    }
    Err(())
}

fn atomic_batch_applied(results: Vec<D1Result>) -> Result<bool> {
    if results.len() != 3 || results.iter().any(|result| !result.success()) {
        return Err(Error::RustError("atomic database command failed".into()));
    }
    let changes = results
        .iter()
        .map(|result| {
            result
                .meta()?
                .and_then(|meta| meta.changes)
                .ok_or_else(|| Error::RustError("database change metadata is unavailable".into()))
        })
        .collect::<Result<Vec<_>>>()?;
    classify_atomic_changes(&changes)
        .map_err(|()| Error::RustError("atomic database command was partially applied".into()))
}

fn json_response<T: Serialize>(value: &T, status: u16, location: Option<&str>) -> Result<Response> {
    let mut response = Response::from_json(value)?.with_status(status);
    if let Some(location) = location {
        response.headers_mut().set("location", location)?;
    }
    Ok(response)
}

fn stored_json_response(status: u16, json: &str) -> Result<Response> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| Error::RustError("stored command response is invalid".into()))?;
    let location = value
        .get("upload_path")
        .or_else(|| value.get("status_path"))
        .or_else(|| value.get("public_share_path"))
        .and_then(serde_json::Value::as_str);
    json_response(&value, status, location)
}

const fn mutation_disabled_failure() -> ApiFailure {
    ApiFailure::new(
        503,
        "mutation_authority_disabled",
        "Mutations are disabled for the current authority phase.",
        true,
    )
}

const fn idempotency_conflict_failure() -> ApiFailure {
    ApiFailure::new(
        409,
        "idempotency_conflict",
        "The idempotency key was already used for a different command.",
        false,
    )
}

const fn revision_conflict_failure() -> ApiFailure {
    ApiFailure::new(
        409,
        "revision_conflict",
        "The video changed before the privacy update was applied.",
        true,
    )
}

const fn storage_unavailable_failure() -> ApiFailure {
    ApiFailure::new(
        503,
        "storage_unavailable",
        "Storage is temporarily unavailable.",
        true,
    )
}

async fn public_share_response(
    env: &Env,
    share_id: &str,
    canonical_origin: &str,
) -> Result<Response> {
    let summary = if valid_uuid(share_id) {
        public_share_row(env, share_id)
            .await?
            .as_ref()
            .map_or_else(unavailable_share, |row| {
                public_summary(row, canonical_origin)
            })
    } else {
        unavailable_share()
    };
    Response::from_json(&summary)
}

async fn public_share_row(env: &Env, share_id: &str) -> Result<Option<PublicShareRow>> {
    env.d1("DB")?
        .prepare(
            "SELECT v.id, v.title, v.state, v.privacy, v.organization_id, \
                    v.playback_object_key, v.duration_ms, om.content_type, om.bytes \
             FROM videos v \
             LEFT JOIN object_manifests om \
               ON om.object_key = v.playback_object_key AND om.state = 'available' \
             WHERE v.id = ?1 AND v.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(share_id)])?
        .first::<PublicShareRow>(None)
        .await
}

fn unavailable_share() -> PublicShareSummary {
    PublicShareSummary {
        api_version: ApiVersion::current(),
        availability: ShareAvailability::Unavailable,
        title: None,
        description: None,
        canonical_url: None,
        duration_ms: None,
        playback: None,
    }
}

fn public_summary(row: &PublicShareRow, canonical_origin: &str) -> PublicShareSummary {
    let canonical_url = format!("{canonical_origin}/s/{}", row.id);
    if !matches!(row.privacy.as_str(), "public" | "unlisted") {
        return unavailable_share();
    }
    if row.state == "processing" {
        return PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Processing,
            title: None,
            description: None,
            canonical_url: Some(canonical_url),
            duration_ms: None,
            playback: None,
        };
    }
    let Some(object) = validated_public_object(row) else {
        return unavailable_share();
    };
    let duration_ms = match row.duration_ms {
        Some(value) if (0..=86_400_000).contains(&value) => u64::try_from(value).ok(),
        None => None,
        Some(_) => return unavailable_share(),
    };
    PublicShareSummary {
        api_version: ApiVersion::current(),
        availability: ShareAvailability::Public,
        title: Some(sanitized_public_title(&row.title)),
        description: None,
        canonical_url: Some(canonical_url),
        duration_ms,
        playback: Some(PlaybackDescriptor {
            path: format!("/api/v1/public/shares/{}/media", row.id),
            content_type: object.content_type,
            supports_range: true,
            captions: Vec::<CaptionTrack>::new(),
        }),
    }
}

fn validated_public_object(row: &PublicShareRow) -> Option<PublicObject> {
    if row.state != "ready" || !matches!(row.privacy.as_str(), "public" | "unlisted") {
        return None;
    }
    let organization_id = row.organization_id.as_deref().filter(|id| valid_uuid(id))?;
    let key = row.playback_object_key.as_deref()?;
    let expected_prefix = format!("tenants/{organization_id}/videos/{}/", row.id);
    if !key.starts_with(&expected_prefix)
        || !key.contains("/derivatives/")
        || key.contains("..")
        || key.contains(['\\', '?', '#', '%'])
    {
        return None;
    }
    let content_type = row
        .content_type
        .as_deref()
        .filter(|value| valid_content_type(value) && value.starts_with("video/"))?;
    let bytes = u64::try_from(row.bytes?).ok()?;
    if bytes == 0 || bytes > MAX_SAFE_INTEGER {
        return None;
    }
    Some(PublicObject {
        key: key.to_owned(),
        content_type: content_type.to_owned(),
        bytes,
    })
}

async fn public_media_response(
    env: &Env,
    request: &Request,
    share_id: &str,
    head_only: bool,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    if !valid_uuid(share_id) {
        return failure_response(not_found_failure(), request_id, production);
    }
    let Some(row) = public_share_row(env, share_id).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(public) = validated_public_object(&row) else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let bucket = env.bucket("RECORDINGS")?;
    let Some(head) = bucket.head(&public.key).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    if head.size() != public.bytes {
        return failure_response(
            media_unavailable_failure("media_unavailable"),
            request_id,
            production,
        );
    }
    let requested_range =
        match parse_range_header(request.headers().get("range")?.as_deref(), public.bytes) {
            Ok(range) => range,
            Err(()) => return range_not_satisfiable(public.bytes, request_id, production),
        };
    let etag = head.http_etag();
    if requested_range.is_none()
        && request
            .headers()
            .get("if-none-match")?
            .is_some_and(|candidate| candidate.trim() == etag)
    {
        let mut response = Response::empty()?.with_status(304);
        response.headers_mut().set("etag", &etag)?;
        return secure_response(response, request_id, production);
    }

    if head_only {
        return media_response(
            Response::empty()?,
            &public,
            &etag,
            requested_range.as_ref(),
            request_id,
            production,
        );
    }
    let object = match requested_range.as_ref() {
        Some(range) => {
            bucket
                .get(&public.key)
                .range(range.range.clone())
                .execute()
                .await?
        }
        None => bucket.get(&public.key).execute().await?,
    };
    let Some(object) = object.filter(|object| object.size() == public.bytes) else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let object_etag = object.http_etag();
    if object_etag != etag {
        return failure_response(
            media_unavailable_failure("media_changed"),
            request_id,
            production,
        );
    }
    let body = object
        .body()
        .ok_or_else(|| Error::RustError("R2 returned no media body".into()))?
        .response_body()?;
    media_response(
        Response::from_body(body)?,
        &public,
        &etag,
        requested_range.as_ref(),
        request_id,
        production,
    )
}

const fn media_unavailable_failure(code: &'static str) -> ApiFailure {
    ApiFailure::new(503, code, "The media is temporarily unavailable.", true)
}

fn media_response(
    mut response: Response,
    public: &PublicObject,
    etag: &str,
    range: Option<&RequestedRange>,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let content_length = range.map_or(public.bytes, |range| range.length);
    if let Some(range) = range {
        response = response.with_status(206);
        response.headers_mut().set(
            "content-range",
            &format!(
                "bytes {}-{}/{}",
                range.start,
                range.start + range.length - 1,
                public.bytes
            ),
        )?;
    }
    let headers = response.headers_mut();
    headers.set("accept-ranges", "bytes")?;
    headers.set("content-length", &content_length.to_string())?;
    headers.set("content-type", &public.content_type)?;
    headers.set("content-disposition", "inline")?;
    headers.set("etag", etag)?;
    secure_response(response, request_id, production)
}

fn range_not_satisfiable(bytes: u64, request_id: &str, production: bool) -> Result<Response> {
    let mut response = failure_response(
        ApiFailure::new(
            416,
            "range_not_satisfiable",
            "The requested byte range is not satisfiable.",
            false,
        ),
        request_id,
        production,
    )?;
    response
        .headers_mut()
        .set("content-range", &format!("bytes */{bytes}"))?;
    Ok(response)
}

fn parse_range_header(
    value: Option<&str>,
    size: u64,
) -> std::result::Result<Option<RequestedRange>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let range = value.strip_prefix("bytes=").ok_or(())?;
    if range.contains(',') || range.bytes().any(|byte| byte.is_ascii_whitespace()) || size == 0 {
        return Err(());
    }
    let (start, end) = range.split_once('-').ok_or(())?;
    if start.is_empty() {
        let requested = end.parse::<u64>().map_err(|_| ())?;
        if requested == 0 {
            return Err(());
        }
        let length = requested.min(size);
        return Ok(Some(RequestedRange {
            range: worker::Range::Suffix { suffix: length },
            start: size - length,
            length,
        }));
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= size || start > MAX_SAFE_INTEGER {
        return Err(());
    }
    let requested_end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(size - 1)
    };
    if requested_end < start || requested_end > MAX_SAFE_INTEGER {
        return Err(());
    }
    let length = requested_end - start + 1;
    let range = if end.is_empty() {
        worker::Range::OffsetToEnd { offset: start }
    } else {
        worker::Range::OffsetWithLength {
            offset: start,
            length,
        }
    };
    Ok(Some(RequestedRange {
        range,
        start,
        length,
    }))
}

async fn authority_response(env: &Env) -> Result<Response> {
    let row = env
        .d1("DB")?
        .prepare("SELECT phase, authority, epoch FROM authority_state WHERE singleton = 1")
        .first::<AuthorityRow>(None)
        .await?
        .ok_or_else(|| Error::RustError("authority state is unavailable".into()))?;
    if !matches!(
        row.phase.as_str(),
        "legacy_authoritative"
            | "shadow_read"
            | "dual_write"
            | "d1_authoritative"
            | "rolled_back"
            | "finalized"
    ) || !matches!(row.authority.as_str(), "legacy" | "dual_write" | "d1")
        || !(0..=i64::try_from(MAX_SAFE_INTEGER).expect("safe integer fits i64"))
            .contains(&row.epoch)
    {
        return Err(Error::RustError("authority state is invalid".into()));
    }
    // This Worker has no legacy adapter. Dual-write is therefore deliberately
    // fail-closed until both authorities and durable outcome reconciliation exist.
    let mutations_enabled = d1_mutation_pair(&row);
    Response::from_json(&AuthorityResponse {
        schema_version: API_SCHEMA_VERSION,
        phase: row.phase,
        authority: row.authority,
        epoch: u64::try_from(row.epoch)
            .map_err(|_| Error::RustError("authority epoch is invalid".into()))?,
        mutations_enabled,
    })
}

async fn authenticated_command_preflight(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    required: RequiredAccess,
) -> Result<std::result::Result<AuthenticatedActor, ApiFailure>> {
    if request.headers().get("cookie").ok().flatten().is_some() {
        return Ok(Err(ApiFailure::new(
            401,
            "unsupported_authentication",
            "This endpoint requires explicit bearer authentication.",
            false,
        )
        .with_authenticate()));
    }
    if request
        .headers()
        .get("origin")
        .ok()
        .flatten()
        .is_some_and(|origin| {
            !origin_allowed(
                &origin,
                &config.host_policy.public_host,
                config.host_policy.deployment == Deployment::Local,
            )
        })
    {
        return Ok(Err(ApiFailure::new(
            403,
            "origin_forbidden",
            "The request origin is not permitted.",
            false,
        )));
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .ok()
        .flatten()
        .is_some_and(|fetch_site| !matches!(fetch_site.as_str(), "same-origin" | "none"))
    {
        return Ok(Err(ApiFailure::new(
            403,
            "origin_forbidden",
            "The request origin is not permitted.",
            false,
        )));
    }

    let Some(authorization) = request
        .headers()
        .get("authorization")
        .map_err(|_| Error::RustError("authorization header is unavailable".into()))?
    else {
        return Ok(Err(unauthenticated_failure()));
    };
    let Some(token) = authorization.strip_prefix("Bearer ").filter(|token| {
        (32..=512).contains(&token.len())
            && token
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
    }) else {
        return Ok(Err(unauthenticated_failure()));
    };
    let now = current_time_ms()?;
    let digest = digest_credential(token);
    let Some(row) = env
        .d1("DB")?
        .prepare(
            "SELECT k.user_id, k.scopes_json FROM auth_api_keys k \
             JOIN users u ON u.id = k.user_id \
             WHERE k.key_digest = ?1 AND k.revoked_at_ms IS NULL \
               AND (k.expires_at_ms IS NULL OR k.expires_at_ms > ?2) \
               AND u.status = 'active' AND u.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(&digest), JsValue::from_f64(now as f64)])?
        .first::<ApiKeyRow>(None)
        .await?
    else {
        return Ok(Err(unauthenticated_failure()));
    };
    if !valid_uuid(&row.user_id) {
        return Err(Error::RustError("authenticated actor is invalid".into()));
    }
    let scopes = serde_json::from_str::<Vec<String>>(&row.scopes_json)
        .map_err(|_| Error::RustError("API key scopes are invalid".into()))?;
    if scopes.is_empty()
        || scopes.len() > 16
        || scopes
            .iter()
            .any(|scope| scope.len() > 64 || !scope.is_ascii())
    {
        return Err(Error::RustError("API key scopes are invalid".into()));
    }
    let actor = AuthenticatedActor {
        user_id: row.user_id,
        scopes,
    };
    if !actor.allows(required) {
        return Ok(Err(ApiFailure::new(
            403,
            "insufficient_scope",
            "The credential does not permit this operation.",
            false,
        )));
    }
    Ok(Ok(actor))
}

async fn authorized_tenant(
    database: &D1Database,
    request: &Request,
    actor: &AuthenticatedActor,
    required: RequiredAccess,
) -> Result<Option<String>> {
    if !actor.allows(required) {
        return Ok(None);
    }
    let Some(tenant_id) = tenant_header(request)? else {
        return Ok(None);
    };
    let Some(membership) = database
        .prepare(
            "SELECT m.role FROM organization_members m \
             JOIN organizations o ON o.id = m.organization_id \
             WHERE m.organization_id = ?1 AND m.user_id = ?2 \
               AND m.state = 'active' AND o.status = 'active' LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&tenant_id),
            JsValue::from_str(&actor.user_id),
        ])?
        .first::<MembershipRow>(None)
        .await?
    else {
        return Ok(None);
    };
    let permitted = match required {
        RequiredAccess::Read => matches!(
            membership.role.as_str(),
            "owner" | "admin" | "member" | "viewer"
        ),
        RequiredAccess::Write => matches!(membership.role.as_str(), "owner" | "admin" | "member"),
        RequiredAccess::Admin => matches!(membership.role.as_str(), "owner" | "admin"),
    };
    Ok(permitted.then_some(tenant_id))
}

fn validate_json_command_headers(request: &Request) -> std::result::Result<(), ApiFailure> {
    validate_idempotency_header(request)?;
    let content_type = request
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("unsupported_content_type"))?;
    if !matches!(
        content_type.as_str(),
        "application/json" | "application/json; charset=utf-8"
    ) {
        return Err(invalid_body_failure("unsupported_content_type"));
    }
    if request
        .headers()
        .get("content-encoding")
        .ok()
        .flatten()
        .is_some_and(|encoding| encoding != "identity")
    {
        return Err(invalid_body_failure("unsupported_content_encoding"));
    }
    let content_length = request
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .ok_or_else(|| {
            ApiFailure::new(
                411,
                "content_length_required",
                "A bounded content length is required.",
                false,
            )
        })?
        .parse::<u64>()
        .map_err(|_| invalid_body_failure("invalid_content_length"))?;
    if content_length == 0 || content_length > MAX_COMMAND_BODY_BYTES {
        return Err(ApiFailure::new(
            413,
            "payload_too_large",
            "The request body exceeds the allowed size.",
            false,
        ));
    }
    Ok(())
}

fn validate_idempotency_header(request: &Request) -> std::result::Result<(), ApiFailure> {
    let key = request
        .headers()
        .get("idempotency-key")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("missing_idempotency_key"))?;
    if !valid_idempotency_key(&key) {
        return Err(invalid_body_failure("invalid_idempotency_key"));
    }
    Ok(())
}

fn invalid_body_failure(code: &'static str) -> ApiFailure {
    ApiFailure::new(
        code_status(code),
        code,
        "The request body is invalid.",
        false,
    )
}

fn code_status(code: &str) -> u16 {
    match code {
        "invalid_schema_version" => 422,
        _ => 400,
    }
}

const fn invalid_identifier_failure() -> ApiFailure {
    ApiFailure::new(
        404,
        "not_found",
        "The requested resource was not found.",
        false,
    )
}

const fn not_found_failure() -> ApiFailure {
    ApiFailure::new(
        404,
        "not_found",
        "The requested resource was not found.",
        false,
    )
}

const fn unauthenticated_failure() -> ApiFailure {
    ApiFailure::new(
        401,
        "unauthenticated",
        "Valid authentication is required.",
        false,
    )
    .with_authenticate()
}

fn failure_response(failure: ApiFailure, request_id: &str, production: bool) -> Result<Response> {
    let mut response = Response::from_json(&ApiError {
        code: failure.code.into(),
        message: failure.message.into(),
        request_id: Some(request_id.into()),
        retry: if failure.retryable {
            RetryAdvice::Later
        } else {
            RetryAdvice::Never
        },
    })?
    .with_status(failure.status);
    if let Some(allow) = failure.allow {
        response.headers_mut().set("allow", allow)?;
    }
    if failure.authenticate {
        response
            .headers_mut()
            .set("www-authenticate", "Bearer realm=\"frame\"")?;
    }
    secure_response(response, request_id, production)
}

fn secure_response(mut response: Response, request_id: &str, production: bool) -> Result<Response> {
    let headers = response.headers_mut();
    headers.set("cache-control", "no-store, max-age=0")?;
    headers.set("pragma", "no-cache")?;
    headers.set("expires", "0")?;
    headers.set("vary", "Origin")?;
    headers.set("x-request-id", request_id)?;
    headers.set("x-content-type-options", "nosniff")?;
    headers.set("x-frame-options", "DENY")?;
    headers.set("referrer-policy", "no-referrer")?;
    headers.set("cross-origin-resource-policy", "same-origin")?;
    headers.set("x-robots-tag", "noindex, nofollow, noarchive")?;
    headers.set(
        "permissions-policy",
        "camera=(), microphone=(), display-capture=(), geolocation=()",
    )?;
    headers.set(
        "content-security-policy",
        "default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
    )?;
    if production {
        headers.set(
            "strict-transport-security",
            "max-age=31536000; includeSubDomains",
        )?;
    }
    Ok(response)
}

fn request_id(request: &Request) -> String {
    let ray = request.headers().get("cf-ray").ok().flatten();
    normalize_cf_ray(
        ray.as_deref(),
        js_sys::Date::now().to_bits(),
        js_sys::Math::random().to_bits(),
    )
}

#[cfg(test)]
mod tests {
    use frame_client::FrameOrigin;

    use super::*;
    use crate::contracts::ValidationCode;

    #[test]
    fn failures_have_stable_status_and_do_not_carry_internal_details() {
        let failure = unauthenticated_failure();
        assert_eq!(failure.status, 401);
        assert_eq!(failure.code, "unauthenticated");
        assert_eq!(failure.message, "Valid authentication is required.");
        assert!(failure.authenticate);
        assert!(!failure.retryable);
    }

    #[test]
    fn validation_codes_map_to_stable_public_statuses() {
        for code in [
            ValidationCode::Identifier,
            ValidationCode::Size,
            ValidationCode::ContentType,
            ValidationCode::ObjectRole,
            ValidationCode::ObjectVersion,
            ValidationCode::Profile,
            ValidationCode::Title,
            ValidationCode::Privacy,
            ValidationCode::Revision,
        ] {
            let failure = invalid_body_failure(code.as_str());
            assert_eq!(failure.status, 400);
            assert_eq!(failure.message, "The request body is invalid.");
        }
        assert_eq!(
            invalid_body_failure(ValidationCode::SchemaVersion.as_str()).status,
            422
        );
    }

    #[test]
    fn capability_discovery_describes_persisted_mutation_transports() {
        let capabilities = CapabilitiesResponse::default();
        assert_eq!(
            capabilities.upload_intents,
            "authenticated_d1_r2_single_put"
        );
        assert_eq!(
            capabilities.media_jobs,
            "fail_closed_pending_runtime_selection"
        );
        assert_eq!(capabilities.media_executor_selection, "server_controlled");
        assert!(!capabilities.managed_stream_library);
    }

    #[test]
    fn authority_pairs_fail_closed_without_a_legacy_dual_writer() {
        let row = |phase: &str, authority: &str| AuthorityRow {
            phase: phase.into(),
            authority: authority.into(),
            epoch: 4,
        };
        assert!(d1_mutation_pair(&row("d1_authoritative", "d1")));
        assert!(d1_mutation_pair(&row("finalized", "d1")));
        assert!(!d1_mutation_pair(&row("dual_write", "dual_write")));
        assert!(!d1_mutation_pair(&row("dual_write", "d1")));
        assert!(!d1_mutation_pair(&row("finalized", "dual_write")));
    }

    #[test]
    fn atomic_command_batches_accept_only_all_or_nothing_effects() {
        assert_eq!(classify_atomic_changes(&[1, 1, 1]), Ok(true));
        assert_eq!(classify_atomic_changes(&[0, 0, 0]), Ok(false));
        assert_eq!(classify_atomic_changes(&[1, 0, 1]), Err(()));
        assert_eq!(classify_atomic_changes(&[]), Err(()));
    }

    #[test]
    fn credential_scopes_and_media_types_are_explicit() {
        let read = AuthenticatedActor {
            user_id: "018f47a6-7b1c-7f55-8f39-8f8a86900101".into(),
            scopes: vec!["frame:read".into()],
        };
        assert!(read.allows(RequiredAccess::Read));
        assert!(!read.allows(RequiredAccess::Write));
        assert!(!read.allows(RequiredAccess::Admin));
        assert!(supported_source_content_type("video/webm"));
        assert!(supported_source_content_type("video/mp4"));
        assert!(!supported_source_content_type("text/html"));
        assert!(!supported_source_content_type("application/octet-stream"));
    }

    fn public_row() -> PublicShareRow {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a8690f123";
        let video = "018f47a6-7b1c-7f55-8f39-8f8a8690f124";
        PublicShareRow {
            id: video.into(),
            title: "Synthetic public recording".into(),
            state: "ready".into(),
            privacy: "public".into(),
            organization_id: Some(tenant.into()),
            playback_object_key: Some(format!(
                "tenants/{tenant}/videos/{video}/derivatives/playback/v1/video.mp4"
            )),
            duration_ms: Some(42_000),
            content_type: Some("video/mp4".into()),
            bytes: Some(1_024),
        }
    }

    #[test]
    fn worker_health_and_share_are_consumable_by_frame_client() {
        let health = health_contract(ServiceStatus::Ok).expect("health contract");
        let encoded = serde_json::to_vec(&health).expect("encode health");
        let decoded: Health = serde_json::from_slice(&encoded).expect("client health");
        decoded.validate().expect("valid client health");

        let summary = public_summary(&public_row(), "https://frame.engmanager.xyz");
        let encoded = serde_json::to_vec(&summary).expect("encode share");
        let decoded: PublicShareSummary = serde_json::from_slice(&encoded).expect("client share");
        decoded
            .validate(&FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("origin"))
            .expect("valid client share");
        assert_eq!(decoded.availability, ShareAvailability::Public);
    }

    #[test]
    fn non_public_and_invalid_object_rows_are_indistinguishable() {
        let mut private = public_row();
        private.privacy = "private".into();
        let mut malformed = public_row();
        malformed.playback_object_key = Some("tenants/other/private.mp4".into());
        let unavailable = serde_json::to_vec(&unavailable_share()).expect("unavailable");
        assert_eq!(
            serde_json::to_vec(&public_summary(&private, "https://frame.engmanager.xyz"))
                .expect("private"),
            unavailable
        );
        assert_eq!(
            serde_json::to_vec(&public_summary(&malformed, "https://frame.engmanager.xyz"))
                .expect("malformed"),
            unavailable
        );
    }

    #[test]
    fn range_parser_accepts_one_bounded_range_and_rejects_ambiguity() {
        let prefix = parse_range_header(Some("bytes=0-9"), 100)
            .expect("range")
            .expect("present");
        assert_eq!((prefix.start, prefix.length), (0, 10));
        assert!(matches!(
            prefix.range,
            worker::Range::OffsetWithLength {
                offset: 0,
                length: 10
            }
        ));
        let tail = parse_range_header(Some("bytes=-12"), 100)
            .expect("suffix")
            .expect("present");
        assert_eq!((tail.start, tail.length), (88, 12));
        let open = parse_range_header(Some("bytes=90-"), 100)
            .expect("open")
            .expect("present");
        assert_eq!((open.start, open.length), (90, 10));
        for invalid in ["bytes=100-", "bytes=9-2", "bytes=0-1,4-5", "bytes=-0"] {
            assert!(parse_range_header(Some(invalid), 100).is_err(), "{invalid}");
        }
    }
}
