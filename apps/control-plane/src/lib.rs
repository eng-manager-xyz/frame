mod contracts;
mod routing;

use contracts::{
    API_SCHEMA_VERSION, AuthorityResponse, CapabilitiesResponse, DiscoveryResponse,
    MAX_COMMAND_BODY_BYTES, MAX_SAFE_INTEGER, MediaJobRequest, UploadIntentRequest,
    constant_time_eq, normalize_cf_ray, origin_allowed, sanitized_public_title, valid_content_type,
    valid_idempotency_key, valid_uuid,
};
use frame_client::{
    ApiError, ApiVersion, Capabilities, CaptionTrack, Health, PlaybackDescriptor,
    PublicShareSummary, RetryAdvice, ServiceStatus, ShareAvailability,
};
use routing::{
    Deployment, HostPolicy, Route, classify_raw_path, parse_raw_request_target, validate_host,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use worker::*;

const PRODUCTION_HOST: &str = "frame.engmanager.xyz";
const INTERNAL_TOKEN_BINDING: &str = "FRAME_INTERNAL_API_TOKEN";

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
                Response::from_json(&CapabilitiesResponse::default())?
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
        Route::UploadIntent => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                if let Err(failure) = authenticated_command_preflight(&request, env, config) {
                    return failure_response(failure, request_id, config.production());
                }
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
                failure_response(contract_stub_failure(), request_id, config.production())?
            }
        }
        Route::MediaJobCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                if let Err(failure) = authenticated_command_preflight(&request, env, config) {
                    return failure_response(failure, request_id, config.production());
                }
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
                failure_response(contract_stub_failure(), request_id, config.production())?
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
                if let Err(failure) = authenticated_command_preflight(&request, env, config) {
                    return failure_response(failure, request_id, config.production());
                }
                failure_response(contract_stub_failure(), request_id, config.production())?
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
                if let Err(failure) = authenticated_command_preflight(&request, env, config) {
                    return failure_response(failure, request_id, config.production());
                }
                if let Err(failure) = validate_idempotency_header(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                failure_response(contract_stub_failure(), request_id, config.production())?
            }
        }
        Route::AuthorityStatus => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                if let Err(failure) = authenticated_command_preflight(&request, env, config) {
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
    let media_transformations = config.media_mode == MediaMode::Fake || has_binding(env, "MEDIA");

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
    Response::from_json(&AuthorityResponse {
        schema_version: API_SCHEMA_VERSION,
        phase: row.phase,
        authority: row.authority,
        epoch: u64::try_from(row.epoch)
            .map_err(|_| Error::RustError("authority epoch is invalid".into()))?,
        mutations_enabled: false,
    })
}

fn authenticated_command_preflight(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
) -> std::result::Result<(), ApiFailure> {
    if request.headers().get("cookie").ok().flatten().is_some() {
        return Err(ApiFailure::new(
            401,
            "unsupported_authentication",
            "This endpoint requires explicit bearer authentication.",
            false,
        )
        .with_authenticate());
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
        return Err(ApiFailure::new(
            403,
            "origin_forbidden",
            "The request origin is not permitted.",
            false,
        ));
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .ok()
        .flatten()
        .is_some_and(|fetch_site| !matches!(fetch_site.as_str(), "same-origin" | "none"))
    {
        return Err(ApiFailure::new(
            403,
            "origin_forbidden",
            "The request origin is not permitted.",
            false,
        ));
    }

    let expected = env
        .secret(INTERNAL_TOKEN_BINDING)
        .map_err(|_| contract_stub_failure())?
        .to_string();
    let authorization = request
        .headers()
        .get("authorization")
        .ok()
        .flatten()
        .ok_or_else(unauthenticated_failure)?;
    let token = authorization
        .strip_prefix("Bearer ")
        .filter(|token| {
            (32..=512).contains(&token.len())
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
        })
        .ok_or_else(unauthenticated_failure)?;
    if !constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        return Err(unauthenticated_failure());
    }
    Ok(())
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

const fn contract_stub_failure() -> ApiFailure {
    ApiFailure::new(
        501,
        "capability_unavailable",
        "The contract is available but mutation is not enabled.",
        false,
    )
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

fn has_binding(env: &Env, name: &str) -> bool {
    js_sys::Reflect::get(env, &JsValue::from_str(name)).is_ok_and(|binding| !binding.is_undefined())
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
    fn capability_discovery_never_claims_mutations_are_live() {
        let capabilities = CapabilitiesResponse::default();
        assert_eq!(capabilities.upload_intents, "authenticated_contract_stub");
        assert_eq!(capabilities.media_jobs, "authenticated_contract_stub");
        assert_eq!(capabilities.media_executor_selection, "server_controlled");
        assert!(!capabilities.managed_stream_library);
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
