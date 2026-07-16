use std::{env, fmt, net::IpAddr, time::Duration};

use anyhow::{Result, bail};
use reqwest::{
    Client, Method, Response, StatusCode,
    header::{
        ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
        LOCATION,
    },
    redirect::Policy,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const API_SCHEMA_VERSION: u16 = 1;
const MAX_JSON_BYTES: usize = 64 * 1_024;
pub(crate) const MAX_SOURCE_BYTES: u64 = 100 * 1_024 * 1_024;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 32 * 1_024 * 1_024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiFailureDisposition {
    Retryable,
    Permanent,
    Cancelled,
    LeaseLost,
    Configuration,
}

#[derive(Debug)]
pub(crate) struct ProtocolApiError {
    code: String,
    disposition: ApiFailureDisposition,
}

impl ProtocolApiError {
    pub(crate) fn new(code: &str, disposition: ApiFailureDisposition) -> Self {
        Self {
            code: code.into(),
            disposition,
        }
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) const fn disposition(&self) -> ApiFailureDisposition {
        self.disposition
    }
}

impl fmt::Display for ProtocolApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "worker protocol rejected the request ({})",
            self.code
        )
    }
}

impl std::error::Error for ProtocolApiError {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiErrorBody {
    code: String,
    message: String,
    request_id: Option<String>,
    retry: RetryAdvice,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum RetryAdvice {
    Later,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerEnvironment {
    Local,
    Production,
}

#[derive(Clone)]
pub(crate) struct WorkerConfig {
    origin: Url,
    token: String,
    tenant_id: String,
    environment: WorkerEnvironment,
}

impl fmt::Debug for WorkerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerConfig")
            .field("origin", &self.origin.as_str())
            .field("token", &"<redacted>")
            .field("tenant_id", &self.tenant_id)
            .field("environment", &self.environment)
            .finish()
    }
}

impl WorkerConfig {
    pub(crate) fn from_env_required() -> Result<Self> {
        Self::from_env_optional()?.ok_or_else(|| {
            anyhow::anyhow!("worker protocol configuration is required for the work-once command")
        })
    }

    pub(crate) fn from_env_optional() -> Result<Option<Self>> {
        let origin = env::var("FRAME_CONTROL_PLANE_ORIGIN").ok();
        let token = env::var("FRAME_MEDIA_WORKER_TOKEN").ok();
        let tenant_id = env::var("FRAME_MEDIA_TENANT_ID").ok();
        if origin.is_none() && token.is_none() && tenant_id.is_none() {
            return Ok(None);
        }
        let origin = origin.ok_or_else(|| {
            anyhow::anyhow!("FRAME_CONTROL_PLANE_ORIGIN is required when the consumer is enabled")
        })?;
        let token = token.ok_or_else(|| {
            anyhow::anyhow!("FRAME_MEDIA_WORKER_TOKEN is required when the consumer is enabled")
        })?;
        let tenant_id = tenant_id.ok_or_else(|| {
            anyhow::anyhow!("FRAME_MEDIA_TENANT_ID is required when the consumer is enabled")
        })?;
        let environment = match env::var("FRAME_MEDIA_WORKER_ENV")
            .unwrap_or_else(|_| "local".into())
            .as_str()
        {
            "local" => WorkerEnvironment::Local,
            "production" => WorkerEnvironment::Production,
            _ => bail!("FRAME_MEDIA_WORKER_ENV must be local or production"),
        };
        Self::new(&origin, &token, &tenant_id, environment).map(Some)
    }

    fn new(
        origin: &str,
        token: &str,
        tenant_id: &str,
        environment: WorkerEnvironment,
    ) -> Result<Self> {
        let mut origin = Url::parse(origin)
            .map_err(|_| anyhow::anyhow!("FRAME_CONTROL_PLANE_ORIGIN is not a valid URL"))?;
        if origin.username() != ""
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !matches!(origin.path(), "" | "/")
        {
            bail!("FRAME_CONTROL_PLANE_ORIGIN must contain only a trusted origin");
        }
        let host = origin
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("FRAME_CONTROL_PLANE_ORIGIN must include a host"))?;
        let local_host = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        match (environment, origin.scheme(), local_host) {
            (_, "https", _) | (WorkerEnvironment::Local, "http", true) => {}
            (WorkerEnvironment::Production, _, _) => {
                bail!("production control-plane origins must use HTTPS")
            }
            _ => bail!("local HTTP control-plane origins must use a loopback host"),
        }
        if !(32..=512).contains(&token.len())
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
        {
            bail!("FRAME_MEDIA_WORKER_TOKEN is invalid");
        }
        let tenant = Uuid::parse_str(tenant_id)
            .map_err(|_| anyhow::anyhow!("FRAME_MEDIA_TENANT_ID is invalid"))?;
        if tenant.is_nil() || tenant.to_string() != tenant_id {
            bail!("FRAME_MEDIA_TENANT_ID must be a canonical non-nil UUID");
        }
        origin.set_path("/");
        Ok(Self {
            origin,
            token: token.into(),
            tenant_id: tenant_id.into(),
            environment,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        if !safe_private_path(path) {
            bail!("the control plane returned an unsafe worker path");
        }
        let mut endpoint = self.origin.clone();
        endpoint.set_path(path);
        Ok(endpoint)
    }

    #[cfg(test)]
    pub(crate) fn local_for_test(origin: &str, token: &str, tenant_id: &str) -> Result<Self> {
        Self::new(origin, token, tenant_id, WorkerEnvironment::Local)
    }
}

#[derive(Clone)]
pub(crate) struct LeaseToken(String);

impl fmt::Debug for LeaseToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LeaseToken(<redacted>)")
    }
}

impl LeaseToken {
    fn generate() -> Self {
        Self(format!(
            "{}{}",
            Uuid::now_v7().simple(),
            Uuid::now_v7().simple()
        ))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub(crate) struct WorkerClient {
    http: Client,
    config: WorkerConfig,
}

impl WorkerClient {
    pub(crate) fn new(config: WorkerConfig) -> Result<Self> {
        let http = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent("frame-media-worker/0.1")
            .build()
            .map_err(|_| anyhow::anyhow!("could not initialize the worker protocol client"))?;
        Ok(Self { http, config })
    }

    pub(crate) async fn claim(&self) -> Result<Option<ClaimedJob>> {
        let lease = LeaseToken::generate();
        let request = WorkerClaimRequest {
            schema_version: API_SCHEMA_VERSION,
            tenant_id: self.config.tenant_id.clone(),
        };
        let response = self
            .json_request(
                Method::POST,
                "/api/v1/worker/media-jobs/claim",
                Some(&lease),
                Some(new_idempotency_key()),
                &request,
            )
            .await?;
        reject_redirect(&response)?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let response = expect_status(response, &[StatusCode::OK]).await?;
        let claim: NativeJobClaimResponse = decode_json(response).await.map_err(|_| {
            ProtocolApiError::new("invalid_job_claim", ApiFailureDisposition::Configuration)
        })?;
        validate_claim(&claim).map_err(|_| {
            ProtocolApiError::new("invalid_job_claim", ApiFailureDisposition::Configuration)
        })?;
        Ok(Some(ClaimedJob { claim, lease }))
    }

    pub(crate) async fn download_source(&self, job: &ClaimedJob) -> Result<Vec<u8>> {
        validate_job_path(&job.claim.job_id, &job.claim.source.path, "source")?;
        let response = self
            .request(Method::GET, &job.claim.source.path, Some(&job.lease))?
            .header(ACCEPT, &job.claim.source.content_type)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("source transport failed"))?;
        reject_redirect(&response)?;
        let response = expect_status(response, &[StatusCode::OK]).await?;
        reject_content_encoding(&response).map_err(|_| {
            ProtocolApiError::new("source_manifest_invalid", ApiFailureDisposition::Permanent)
        })?;
        ensure_header_exact(&response, CONTENT_TYPE, &job.claim.source.content_type).map_err(
            |_| ProtocolApiError::new("source_manifest_invalid", ApiFailureDisposition::Permanent),
        )?;
        ensure_header_exact(
            &response,
            "x-content-sha256",
            &job.claim.source.checksum_sha256,
        )
        .map_err(|_| {
            ProtocolApiError::new("source_manifest_invalid", ApiFailureDisposition::Permanent)
        })?;
        let declared_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if declared_length != Some(job.claim.source.bytes) {
            return Err(ProtocolApiError::new(
                "source_manifest_invalid",
                ApiFailureDisposition::Permanent,
            )
            .into());
        }
        let bytes = read_bounded(response, job.claim.source.bytes as usize).await?;
        if bytes.len() as u64 != job.claim.source.bytes
            || sha256_hex(&bytes) != job.claim.source.checksum_sha256
        {
            return Err(ProtocolApiError::new(
                "source_manifest_invalid",
                ApiFailureDisposition::Permanent,
            )
            .into());
        }
        Ok(bytes)
    }

    pub(crate) async fn heartbeat(&self, job: &ClaimedJob) -> Result<WorkerJobResponse> {
        self.job_command(
            job,
            &job.claim.heartbeat_path,
            "heartbeat",
            &WorkerHeartbeatRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: self.config.tenant_id.clone(),
            },
        )
        .await
    }

    pub(crate) async fn progress(
        &self,
        job: &ClaimedJob,
        progress_basis_points: u16,
    ) -> Result<WorkerJobResponse> {
        if progress_basis_points >= 10_000 {
            bail!("worker progress must remain below the terminal value");
        }
        self.job_command(
            job,
            &job.claim.progress_path,
            "progress",
            &WorkerProgressRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: self.config.tenant_id.clone(),
                progress_basis_points,
            },
        )
        .await
    }

    pub(crate) async fn upload_output(
        &self,
        job: &ClaimedJob,
        bytes: Vec<u8>,
        checksum_sha256: &str,
    ) -> Result<WorkerOutputResponse> {
        validate_job_path(&job.claim.job_id, &job.claim.output.path, "output")?;
        if bytes.is_empty()
            || bytes.len() as u64 > job.claim.output.max_bytes
            || bytes.len() as u64 > MAX_OUTPUT_BYTES
            || !valid_sha256(checksum_sha256)
            || sha256_hex(&bytes) != checksum_sha256
        {
            bail!("output failed the bounded immutable transport policy");
        }
        let response = self
            .request(Method::PUT, &job.claim.output.path, Some(&job.lease))?
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, &job.claim.output.content_type)
            .header(CONTENT_LENGTH, bytes.len().to_string())
            .header("x-content-sha256", checksum_sha256)
            .header("idempotency-key", new_idempotency_key())
            .body(bytes)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("output transport failed"))?;
        reject_redirect(&response)?;
        let response = expect_status(response, &[StatusCode::OK, StatusCode::CREATED]).await?;
        let output: WorkerOutputResponse = decode_json(response).await.map_err(|_| {
            ProtocolApiError::new(
                "invalid_output_acknowledgement",
                ApiFailureDisposition::Configuration,
            )
        })?;
        if output.schema_version != API_SCHEMA_VERSION
            || output.job_id != job.claim.job_id
            || !output.accepted
            || output.bytes == 0
            || output.bytes > job.claim.output.max_bytes
            || output.checksum_sha256 != checksum_sha256
            || output.content_type != job.claim.output.content_type
        {
            return Err(ProtocolApiError::new(
                "output_acknowledgement_invalid",
                ApiFailureDisposition::Permanent,
            )
            .into());
        }
        Ok(output)
    }

    pub(crate) async fn complete(
        &self,
        job: &ClaimedJob,
        bytes: u64,
        checksum_sha256: &str,
    ) -> Result<WorkerJobResponse> {
        self.job_command(
            job,
            &job.claim.complete_path,
            "complete",
            &WorkerCompleteRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: self.config.tenant_id.clone(),
                bytes,
                checksum_sha256: checksum_sha256.into(),
                content_type: job.claim.output.content_type.clone(),
            },
        )
        .await
    }

    pub(crate) async fn fail(
        &self,
        job: &ClaimedJob,
        error_class: &str,
        retryable: bool,
    ) -> Result<WorkerJobResponse> {
        self.job_command(
            job,
            &job.claim.fail_path,
            "fail",
            &WorkerFailRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: self.config.tenant_id.clone(),
                error_class: error_class.into(),
                retryable,
            },
        )
        .await
    }

    async fn job_command<T: Serialize>(
        &self,
        job: &ClaimedJob,
        path: &str,
        suffix: &str,
        body: &T,
    ) -> Result<WorkerJobResponse> {
        validate_job_path(&job.claim.job_id, path, suffix)?;
        let response = self
            .json_request(
                Method::POST,
                path,
                Some(&job.lease),
                Some(new_idempotency_key()),
                body,
            )
            .await?;
        reject_redirect(&response)?;
        let response = expect_status(response, &[StatusCode::OK]).await?;
        let state: WorkerJobResponse = decode_json(response).await.map_err(|_| {
            ProtocolApiError::new("invalid_job_response", ApiFailureDisposition::Configuration)
        })?;
        validate_job_state(&state, &job.claim.job_id).map_err(|_| {
            ProtocolApiError::new("invalid_job_response", ApiFailureDisposition::Configuration)
        })?;
        Ok(state)
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        lease: Option<&LeaseToken>,
    ) -> Result<reqwest::RequestBuilder> {
        let endpoint = self.config.endpoint(path)?;
        let mut request = self
            .http
            .request(method, endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.config.token))
            .header("x-frame-tenant-id", &self.config.tenant_id)
            .header(ACCEPT_ENCODING, "identity");
        if let Some(lease) = lease {
            request = request.header("x-frame-lease-token", lease.as_str());
        }
        Ok(request)
    }

    async fn json_request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        lease: Option<&LeaseToken>,
        idempotency_key: Option<String>,
        value: &T,
    ) -> Result<Response> {
        let body = serde_json::to_vec(value)
            .map_err(|_| anyhow::anyhow!("worker command could not be encoded"))?;
        if body.is_empty() || body.len() > MAX_JSON_BYTES {
            bail!("worker command exceeded its bounded body policy");
        }
        let mut request = self
            .request(method, path, lease)?
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_LENGTH, body.len().to_string())
            .body(body);
        if let Some(idempotency_key) = idempotency_key {
            request = request.header("idempotency-key", idempotency_key);
        }
        request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("worker command transport failed"))
    }
}

#[derive(Clone)]
pub(crate) struct ClaimedJob {
    pub(crate) claim: NativeJobClaimResponse,
    lease: LeaseToken,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerSourceDescriptor {
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) checksum_sha256: String,
    pub(crate) content_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerOutputDescriptor {
    pub(crate) path: String,
    pub(crate) content_type: String,
    pub(crate) max_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeJobClaimResponse {
    schema_version: u16,
    pub(crate) job_id: String,
    state: String,
    pub(crate) profile: String,
    attempt: u32,
    revision: u64,
    lease_expires_at_ms: u64,
    pub(crate) source: WorkerSourceDescriptor,
    pub(crate) output: WorkerOutputDescriptor,
    heartbeat_path: String,
    progress_path: String,
    complete_path: String,
    fail_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerJobResponse {
    schema_version: u16,
    job_id: String,
    pub(crate) state: String,
    attempt: u32,
    revision: u64,
    progress_basis_points: Option<u16>,
    pub(crate) cancel_requested: bool,
    lease_expires_at_ms: Option<u64>,
    retry_scheduled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerOutputResponse {
    schema_version: u16,
    job_id: String,
    accepted: bool,
    pub(crate) bytes: u64,
    checksum_sha256: String,
    content_type: String,
}

#[derive(Serialize)]
struct WorkerClaimRequest {
    schema_version: u16,
    tenant_id: String,
}

#[derive(Serialize)]
struct WorkerHeartbeatRequest {
    schema_version: u16,
    tenant_id: String,
}

#[derive(Serialize)]
struct WorkerProgressRequest {
    schema_version: u16,
    tenant_id: String,
    progress_basis_points: u16,
}

#[derive(Serialize)]
struct WorkerCompleteRequest {
    schema_version: u16,
    tenant_id: String,
    bytes: u64,
    checksum_sha256: String,
    content_type: String,
}

#[derive(Serialize)]
struct WorkerFailRequest {
    schema_version: u16,
    tenant_id: String,
    error_class: String,
    retryable: bool,
}

fn validate_claim(claim: &NativeJobClaimResponse) -> Result<()> {
    if claim.schema_version != API_SCHEMA_VERSION
        || !valid_uuid(&claim.job_id)
        || !matches!(claim.state.as_str(), "leased" | "running")
        || claim.profile != "thumbnail_v1"
        || !(1..=64).contains(&claim.attempt)
        || claim.revision == 0
        || claim.lease_expires_at_ms <= current_time_ms()
        || claim.source.bytes == 0
        || claim.source.bytes > MAX_SOURCE_BYTES
        || !valid_sha256(&claim.source.checksum_sha256)
        || !matches!(
            claim.source.content_type.as_str(),
            "video/mp4" | "video/quicktime" | "video/webm" | "video/x-matroska"
        )
        || claim.output.content_type != "image/png"
        || claim.output.max_bytes == 0
        || claim.output.max_bytes > MAX_OUTPUT_BYTES
    {
        bail!("control plane returned an invalid native job claim");
    }
    validate_job_path(&claim.job_id, &claim.source.path, "source")?;
    validate_job_path(&claim.job_id, &claim.output.path, "output")?;
    validate_job_path(&claim.job_id, &claim.heartbeat_path, "heartbeat")?;
    validate_job_path(&claim.job_id, &claim.progress_path, "progress")?;
    validate_job_path(&claim.job_id, &claim.complete_path, "complete")?;
    validate_job_path(&claim.job_id, &claim.fail_path, "fail")?;
    Ok(())
}

fn validate_job_state(state: &WorkerJobResponse, expected_job_id: &str) -> Result<()> {
    if state.schema_version != API_SCHEMA_VERSION
        || state.job_id != expected_job_id
        || !matches!(
            state.state.as_str(),
            "queued" | "leased" | "running" | "succeeded" | "failed" | "cancelled"
        )
        || state.attempt == 0
        || state.revision == 0
        || state
            .progress_basis_points
            .is_some_and(|value| value > 10_000)
        || (matches!(state.state.as_str(), "leased" | "running")
            && state.lease_expires_at_ms.is_none())
        || (state.retry_scheduled && state.state != "queued")
    {
        bail!("control plane returned an invalid worker job state");
    }
    Ok(())
}

fn validate_job_path(job_id: &str, path: &str, suffix: &str) -> Result<()> {
    if path != format!("/api/v1/worker/media-jobs/{job_id}/{suffix}") || !safe_private_path(path) {
        bail!("control plane returned an invalid worker job path");
    }
    Ok(())
}

fn safe_private_path(path: &str) -> bool {
    path.starts_with("/api/v1/worker/media-jobs/")
        && path.is_ascii()
        && !path.contains(['?', '#', '%', '\\'])
        && !path.split('/').any(|segment| matches!(segment, "." | ".."))
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|uuid| !uuid.is_nil() && uuid.to_string() == value)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn new_idempotency_key() -> String {
    format!("worker:{}", Uuid::now_v7())
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn reject_redirect(response: &Response) -> Result<()> {
    if response.status().is_redirection() || response.headers().contains_key(LOCATION) {
        bail!("worker protocol redirects are forbidden");
    }
    Ok(())
}

fn reject_content_encoding(response: &Response) -> Result<()> {
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        bail!("encoded worker transport bodies are forbidden");
    }
    Ok(())
}

fn ensure_header_exact(
    response: &Response,
    name: impl reqwest::header::AsHeaderName,
    expected: &str,
) -> Result<()> {
    if response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        != Some(expected)
    {
        bail!("worker transport metadata did not match its manifest");
    }
    Ok(())
}

async fn expect_status(response: Response, expected: &[StatusCode]) -> Result<Response> {
    if expected.contains(&response.status()) {
        return Ok(response);
    }
    let body = decode_json::<ApiErrorBody>(response).await.map_err(|_| {
        ProtocolApiError::new("malformed_api_error", ApiFailureDisposition::Configuration)
    })?;
    if !valid_api_code(&body.code)
        || body.message.is_empty()
        || body.message.len() > 1_024
        || body.message.chars().any(char::is_control)
        || body.request_id.as_ref().is_some_and(|request_id| {
            request_id.is_empty()
                || request_id.len() > 128
                || !request_id.bytes().all(|byte| byte.is_ascii_graphic())
        })
    {
        return Err(ProtocolApiError::new(
            "malformed_api_error",
            ApiFailureDisposition::Configuration,
        )
        .into());
    }
    let disposition = classify_api_code(&body.code);
    let retry_matches = match disposition {
        ApiFailureDisposition::Retryable | ApiFailureDisposition::LeaseLost => {
            body.retry == RetryAdvice::Later
        }
        ApiFailureDisposition::Permanent
        | ApiFailureDisposition::Cancelled
        | ApiFailureDisposition::Configuration => body.retry == RetryAdvice::Never,
    };
    if !retry_matches {
        return Err(ProtocolApiError::new(
            "api_retry_policy_invalid",
            ApiFailureDisposition::Configuration,
        )
        .into());
    }
    let code = if is_allowlisted_api_code(&body.code) {
        body.code.as_str()
    } else {
        "unrecognized_api_error"
    };
    Err(ProtocolApiError::new(code, disposition).into())
}

fn valid_api_code(code: &str) -> bool {
    (1..=64).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_allowlisted_api_code(code: &str) -> bool {
    !matches!(
        classify_api_code(code),
        ApiFailureDisposition::Configuration
    ) || matches!(
        code,
        "api_retry_policy_invalid"
            | "content_length_required"
            | "forbidden"
            | "idempotency_conflict"
            | "invalid_api_path"
            | "invalid_checksum"
            | "invalid_content_checksum"
            | "invalid_content_length"
            | "invalid_failure_class"
            | "invalid_identifier"
            | "invalid_idempotency_key"
            | "invalid_json"
            | "invalid_lease_token"
            | "invalid_progress"
            | "invalid_schema_version"
            | "missing_checksum"
            | "missing_idempotency_key"
            | "missing_lease_token"
            | "not_found"
            | "origin_forbidden"
            | "payload_too_large"
            | "unauthenticated"
            | "unsupported_authentication"
            | "unsupported_content_encoding"
            | "unsupported_content_type"
    )
}

fn classify_api_code(code: &str) -> ApiFailureDisposition {
    match code {
        "claim_conflict"
        | "internal_error"
        | "media_executor_unavailable"
        | "media_unavailable"
        | "mutation_authority_disabled"
        | "native_worker_unavailable"
        | "output_not_ready"
        | "rate_limited"
        | "revision_conflict"
        | "service_unavailable"
        | "source_not_ready"
        | "storage_unavailable"
        | "upload_reconciliation_required" => ApiFailureDisposition::Retryable,
        "invalid_output_manifest"
        | "output_acknowledgement_invalid"
        | "output_conflict"
        | "output_invalid"
        | "profile_unavailable"
        | "source_invalid"
        | "source_manifest_invalid"
        | "unsupported_media_type"
        | "unsupported_source_media_type" => ApiFailureDisposition::Permanent,
        "cancellation_requested" => ApiFailureDisposition::Cancelled,
        "lease_conflict" => ApiFailureDisposition::LeaseLost,
        _ => ApiFailureDisposition::Configuration,
    }
}

async fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T> {
    if response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        != Some("application/json")
    {
        bail!("worker protocol returned a non-JSON response");
    }
    let body = read_bounded(response, MAX_JSON_BYTES).await?;
    serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("worker protocol returned malformed JSON"))
}

async fn read_bounded(mut response: Response, max_bytes: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        bail!("worker protocol response exceeded its bounded body policy");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("worker protocol response body failed"))?
    {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            bail!("worker protocol response exceeded its bounded body policy");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f123";
    const JOB: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f124";

    #[test]
    fn production_requires_https_and_local_http_is_loopback_only() {
        assert!(
            WorkerConfig::new(
                "https://control.example.com",
                "frame-worker-secret-000000000001",
                TENANT,
                WorkerEnvironment::Production
            )
            .is_ok()
        );
        assert!(
            WorkerConfig::new(
                "http://127.0.0.1:8787",
                "frame-worker-secret-000000000001",
                TENANT,
                WorkerEnvironment::Local
            )
            .is_ok()
        );
        assert!(
            WorkerConfig::new(
                "http://control.example.com",
                "frame-worker-secret-000000000001",
                TENANT,
                WorkerEnvironment::Local
            )
            .is_err()
        );
        assert!(
            WorkerConfig::new(
                "https://secret@control.example.com/path?token=value",
                "frame-worker-secret-000000000001",
                TENANT,
                WorkerEnvironment::Production
            )
            .is_err()
        );
    }

    #[test]
    fn bearer_policy_exactly_matches_the_control_plane() {
        for token in ["a".repeat(32), "z".repeat(512)] {
            assert!(
                WorkerConfig::new(
                    "https://control.example.com",
                    &token,
                    TENANT,
                    WorkerEnvironment::Production,
                )
                .is_ok()
            );
        }
        for token in [
            "a".repeat(31),
            "a".repeat(513),
            format!("{} ", "a".repeat(31)),
            format!("{}\"", "a".repeat(31)),
            format!("{}\\", "a".repeat(31)),
            format!("{}é", "a".repeat(31)),
        ] {
            assert!(
                WorkerConfig::new(
                    "https://control.example.com",
                    &token,
                    TENANT,
                    WorkerEnvironment::Production,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn api_codes_have_bounded_allowlisted_dispositions() {
        assert_eq!(
            classify_api_code("storage_unavailable"),
            ApiFailureDisposition::Retryable
        );
        assert_eq!(
            classify_api_code("output_conflict"),
            ApiFailureDisposition::Permanent
        );
        assert_eq!(
            classify_api_code("cancellation_requested"),
            ApiFailureDisposition::Cancelled
        );
        assert_eq!(
            classify_api_code("lease_conflict"),
            ApiFailureDisposition::LeaseLost
        );
        assert_eq!(
            classify_api_code("future_server_code"),
            ApiFailureDisposition::Configuration
        );
        assert!(valid_api_code("output_invalid"));
        assert!(!valid_api_code(&"a".repeat(65)));
        assert!(!valid_api_code("Output_Invalid"));
        assert!(!is_allowlisted_api_code("future_server_code"));
    }

    #[test]
    fn config_and_lease_debug_output_redact_credentials() {
        let config = WorkerConfig::new(
            "https://control.example.com",
            "frame-worker-secret-000000000001",
            TENANT,
            WorkerEnvironment::Production,
        )
        .expect("config");
        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("frame-worker-secret-000000000001"));
        let lease = LeaseToken::generate();
        let raw = lease.as_str().to_owned();
        assert_eq!(raw.len(), 64);
        assert!(raw.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!format!("{lease:?}").contains(&raw));
    }

    #[test]
    fn worker_paths_are_exact_and_cannot_cross_origins() {
        let exact = format!("/api/v1/worker/media-jobs/{JOB}/source");
        assert!(validate_job_path(JOB, &exact, "source").is_ok());
        assert!(validate_job_path(JOB, "https://attacker.example/source", "source").is_err());
        assert!(
            validate_job_path(
                JOB,
                &format!("/api/v1/worker/media-jobs/{JOB}/../output"),
                "source"
            )
            .is_err()
        );
        assert!(
            validate_job_path(
                JOB,
                &format!("/api/v1/worker/media-jobs/{JOB}/source?token=x"),
                "source"
            )
            .is_err()
        );
    }

    #[test]
    fn claim_rejects_oversized_or_unsafe_descriptors() {
        let mut claim = NativeJobClaimResponse {
            schema_version: 1,
            job_id: JOB.into(),
            state: "leased".into(),
            profile: "thumbnail_v1".into(),
            attempt: 1,
            revision: 2,
            lease_expires_at_ms: current_time_ms() + 60_000,
            source: WorkerSourceDescriptor {
                path: format!("/api/v1/worker/media-jobs/{JOB}/source"),
                bytes: 12,
                checksum_sha256: "a".repeat(64),
                content_type: "video/webm".into(),
            },
            output: WorkerOutputDescriptor {
                path: format!("/api/v1/worker/media-jobs/{JOB}/output"),
                content_type: "image/png".into(),
                max_bytes: 1_024,
            },
            heartbeat_path: format!("/api/v1/worker/media-jobs/{JOB}/heartbeat"),
            progress_path: format!("/api/v1/worker/media-jobs/{JOB}/progress"),
            complete_path: format!("/api/v1/worker/media-jobs/{JOB}/complete"),
            fail_path: format!("/api/v1/worker/media-jobs/{JOB}/fail"),
        };
        assert!(validate_claim(&claim).is_ok());
        claim.source.bytes = MAX_SOURCE_BYTES + 1;
        assert!(validate_claim(&claim).is_err());
    }

    #[test]
    fn sha256_is_lowercase_and_stable() {
        assert_eq!(
            sha256_hex(b"frame"),
            "9dff50df08c635815f4b19da10f756605a34a79a48d4ba48712782502975a70e"
        );
    }
}
