use std::{env, fmt, net::IpAddr, path::Path, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use frame_media::{
    CancellationToken, MEDIA_JOB_CATALOG_VERSION, MEDIA_SERVICE_CATALOG_VERSION,
    media_service_catalog,
};
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

use crate::native::{
    NATIVE_EXECUTION_PLAN_SCHEMA_VERSION, NativeExecutionOriginV1, NativeProfile,
    NativeSandboxEnvelopeV1,
};

const API_SCHEMA_VERSION: u16 = 1;
const MAX_JSON_BYTES: usize = 64 * 1_024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(crate) const MAX_SOURCE_BYTES: u64 = 20_000_000_000;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 20_000_000_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const STREAM_TIMEOUT: Duration = Duration::from_secs(7_200);

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
            .field("tenant_id", &"<redacted>")
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
        Ok(Some(ClaimedJob {
            claim,
            lease,
            command_gate: Arc::new(Mutex::new(())),
        }))
    }

    pub(crate) async fn download_source_to(
        &self,
        job: &ClaimedJob,
        ordinal: usize,
        destination: &Path,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let source = job
            .claim
            .sources()
            .get(ordinal)
            .ok_or_else(|| anyhow::anyhow!("native source ordinal is invalid"))?;
        validate_descriptor_path(
            &job.claim.job_id,
            &source.path,
            "sources",
            ordinal,
            job.claim.uses_plural_descriptors(),
        )?;
        let send = self
            .request(Method::GET, &source.path, Some(&job.lease))?
            .header(ACCEPT, &source.content_type)
            .timeout(STREAM_TIMEOUT)
            .send();
        tokio::pin!(send);
        let response = loop {
            tokio::select! {
                result = &mut send => {
                    break result.map_err(|_| anyhow::anyhow!("source transport failed"))?;
                }
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancellation.is_cancelled() {
                        return Err(ProtocolApiError::new(
                            "cancellation_requested",
                            ApiFailureDisposition::Cancelled,
                        ).into());
                    }
                }
            }
        };
        reject_redirect(&response)?;
        let response = expect_status(response, &[StatusCode::OK]).await?;
        reject_content_encoding(&response).map_err(|_| {
            ProtocolApiError::new("source_manifest_invalid", ApiFailureDisposition::Permanent)
        })?;
        ensure_header_exact(&response, CONTENT_TYPE, &source.content_type).map_err(|_| {
            ProtocolApiError::new("source_manifest_invalid", ApiFailureDisposition::Permanent)
        })?;
        ensure_header_exact(&response, "x-content-sha256", &source.checksum_sha256).map_err(
            |_| ProtocolApiError::new("source_manifest_invalid", ApiFailureDisposition::Permanent),
        )?;
        let declared_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if declared_length != Some(source.bytes) {
            return Err(ProtocolApiError::new(
                "source_manifest_invalid",
                ApiFailureDisposition::Permanent,
            )
            .into());
        }
        let mut response = response;
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .await
            .map_err(|_| anyhow::anyhow!("source scratch file could not be created"))?;
        let transfer = async {
            let mut received = 0_u64;
            let mut digest = Sha256::new();
            loop {
                if cancellation.is_cancelled() {
                    return Err(ProtocolApiError::new(
                        "cancellation_requested",
                        ApiFailureDisposition::Cancelled,
                    )
                    .into());
                }
                let chunk = response.chunk();
                tokio::pin!(chunk);
                let next = loop {
                    tokio::select! {
                        result = &mut chunk => {
                            break result
                                .map_err(|_| anyhow::anyhow!("source response body failed"))?;
                        }
                        () = tokio::time::sleep(Duration::from_millis(100)) => {
                            if cancellation.is_cancelled() {
                                return Err(ProtocolApiError::new(
                                    "cancellation_requested",
                                    ApiFailureDisposition::Cancelled,
                                ).into());
                            }
                        }
                    }
                };
                let Some(chunk) = next else {
                    break;
                };
                let Some(next_received) = received
                    .checked_add(chunk.len() as u64)
                    .filter(|value| *value <= source.bytes && *value <= MAX_SOURCE_BYTES)
                else {
                    return Err(ProtocolApiError::new(
                        "source_manifest_invalid",
                        ApiFailureDisposition::Permanent,
                    )
                    .into());
                };
                received = next_received;
                digest.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .map_err(|_| anyhow::anyhow!("source scratch write failed"))?;
            }
            file.sync_all()
                .await
                .map_err(|_| anyhow::anyhow!("source scratch sync failed"))?;
            let checksum = digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if received != source.bytes || checksum != source.checksum_sha256 {
                return Err(ProtocolApiError::new(
                    "source_manifest_invalid",
                    ApiFailureDisposition::Permanent,
                )
                .into());
            }
            Ok(())
        }
        .await;
        if transfer.is_err() {
            drop(file);
            let _ = tokio::fs::remove_file(destination).await;
        }
        transfer
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

    pub(crate) async fn upload_output_file(
        &self,
        job: &ClaimedJob,
        path: &Path,
        bytes: u64,
        checksum_sha256: &str,
        cancellation: &CancellationToken,
    ) -> Result<WorkerOutputResponse> {
        let output = job
            .claim
            .outputs()
            .first()
            .ok_or_else(|| anyhow::anyhow!("native output descriptor is unavailable"))?;
        validate_descriptor_path(
            &job.claim.job_id,
            &output.path,
            "outputs",
            0,
            job.claim.uses_plural_descriptors(),
        )?;
        if bytes == 0
            || bytes > output.max_bytes
            || bytes > MAX_OUTPUT_BYTES
            || !valid_sha256(checksum_sha256)
        {
            bail!("output failed the bounded immutable transport policy");
        }
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|_| anyhow::anyhow!("native output file is unavailable"))?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != bytes
        {
            bail!("output failed the bounded immutable transport policy");
        }
        let (hashed_bytes, hashed_checksum) =
            sha256_file(path, output.max_bytes, cancellation).await?;
        if hashed_bytes != bytes || hashed_checksum != checksum_sha256 {
            bail!("output failed the bounded immutable transport policy");
        }
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|_| anyhow::anyhow!("native output file could not be opened"))?;
        if cancellation.is_cancelled() {
            return Err(ProtocolApiError::new(
                "cancellation_requested",
                ApiFailureDisposition::Cancelled,
            )
            .into());
        }
        let send = self
            .request(Method::PUT, &output.path, Some(&job.lease))?
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, &output.content_type)
            .header(CONTENT_LENGTH, bytes.to_string())
            .header("x-content-sha256", checksum_sha256)
            .header("idempotency-key", new_idempotency_key())
            .timeout(STREAM_TIMEOUT)
            .body(reqwest::Body::from(file))
            .send();
        tokio::pin!(send);
        let response = loop {
            tokio::select! {
                result = &mut send => {
                    break result.map_err(|_| anyhow::anyhow!("output transport failed"))?;
                }
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancellation.is_cancelled() {
                        return Err(ProtocolApiError::new(
                            "cancellation_requested",
                            ApiFailureDisposition::Cancelled,
                        ).into());
                    }
                }
            }
        };
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
            || output.bytes != bytes
            || output.bytes > MAX_SAFE_INTEGER
            || output.checksum_sha256 != checksum_sha256
            || output.content_type != job.claim.outputs()[0].content_type
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
        let content_type = job
            .claim
            .outputs()
            .first()
            .map(|output| output.content_type.clone())
            .ok_or_else(|| anyhow::anyhow!("native output descriptor is unavailable"))?;
        let max_bytes = job.claim.outputs()[0].max_bytes;
        if bytes == 0
            || bytes > max_bytes
            || bytes > MAX_OUTPUT_BYTES
            || !valid_sha256(checksum_sha256)
        {
            bail!("output failed the bounded immutable completion policy");
        }
        if job.claim.uses_plural_descriptors() {
            self.job_command(
                job,
                &job.claim.complete_path,
                "complete",
                &WorkerCompleteOutputsRequest {
                    schema_version: API_SCHEMA_VERSION,
                    tenant_id: self.config.tenant_id.clone(),
                    outputs: vec![WorkerCompletedOutput {
                        ordinal: 0,
                        bytes,
                        checksum_sha256: checksum_sha256.into(),
                        content_type,
                    }],
                },
            )
            .await
        } else {
            self.job_command(
                job,
                &job.claim.complete_path,
                "complete",
                &WorkerCompleteRequest {
                    schema_version: API_SCHEMA_VERSION,
                    tenant_id: self.config.tenant_id.clone(),
                    bytes,
                    checksum_sha256: checksum_sha256.into(),
                    content_type,
                },
            )
            .await
        }
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
        let _command = job.command_gate.lock().await;
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
        if state.attempt != job.claim.attempt || state.revision < job.claim.revision {
            return Err(ProtocolApiError::new(
                "invalid_job_response",
                ApiFailureDisposition::Configuration,
            )
            .into());
        }
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
    command_gate: Arc<Mutex<()>>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerSourceDescriptor {
    #[serde(default)]
    pub(crate) ordinal: Option<u16>,
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) checksum_sha256: String,
    pub(crate) content_type: String,
}

impl fmt::Debug for WorkerSourceDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerSourceDescriptor")
            .field("ordinal", &self.ordinal)
            .field("path", &"<redacted>")
            .field("bytes", &self.bytes)
            .field("checksum_sha256", &"<redacted>")
            .field("content_type", &self.content_type)
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerOutputDescriptor {
    #[serde(default)]
    pub(crate) ordinal: Option<u16>,
    #[serde(default)]
    role: Option<String>,
    pub(crate) path: String,
    pub(crate) content_type: String,
    pub(crate) max_bytes: u64,
}

impl fmt::Debug for WorkerOutputDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerOutputDescriptor")
            .field("ordinal", &self.ordinal)
            .field("role", &self.role)
            .field("path", &"<redacted>")
            .field("content_type", &self.content_type)
            .field("max_bytes", &self.max_bytes)
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeJobClaimResponse {
    schema_version: u16,
    #[serde(default)]
    native_plan_schema_version: Option<u16>,
    #[serde(default)]
    media_job_catalog_version: Option<u16>,
    #[serde(default)]
    media_service_catalog_version: Option<u16>,
    pub(crate) job_id: String,
    state: String,
    pub(crate) profile: String,
    #[serde(default)]
    execution_origin: Option<NativeExecutionOriginV1>,
    #[serde(default)]
    sandbox: Option<NativeSandboxEnvelopeV1>,
    attempt: u32,
    revision: u64,
    lease_expires_at_ms: u64,
    #[serde(default)]
    source: Option<WorkerSourceDescriptor>,
    #[serde(default)]
    sources: Vec<WorkerSourceDescriptor>,
    #[serde(default)]
    output: Option<WorkerOutputDescriptor>,
    #[serde(default)]
    outputs: Vec<WorkerOutputDescriptor>,
    heartbeat_path: String,
    progress_path: String,
    complete_path: String,
    fail_path: String,
}

impl NativeJobClaimResponse {
    pub(crate) fn sources(&self) -> &[WorkerSourceDescriptor] {
        if self.sources.is_empty() {
            self.source.as_slice()
        } else {
            &self.sources
        }
    }

    pub(crate) fn outputs(&self) -> &[WorkerOutputDescriptor] {
        if self.outputs.is_empty() {
            self.output.as_slice()
        } else {
            &self.outputs
        }
    }

    pub(crate) const fn uses_plural_descriptors(&self) -> bool {
        !self.sources.is_empty() || !self.outputs.is_empty()
    }

    pub(crate) fn native_profile(&self) -> Result<NativeProfile> {
        NativeProfile::parse(&self.profile).map_err(Into::into)
    }
}

#[derive(Clone, Deserialize)]
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

#[derive(Clone, Deserialize)]
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
struct WorkerCompleteOutputsRequest {
    schema_version: u16,
    tenant_id: String,
    outputs: Vec<WorkerCompletedOutput>,
}

#[derive(Serialize)]
struct WorkerCompletedOutput {
    ordinal: u16,
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
    let profile = claim.native_profile()?;
    let spec = media_service_catalog()
        .get(profile.job_kind())
        .ok_or_else(|| anyhow::anyhow!("control plane returned an unsupported native profile"))?;
    let sources = claim.sources();
    let outputs = claim.outputs();
    let plural = claim.uses_plural_descriptors();
    if claim.schema_version != API_SCHEMA_VERSION
        || !valid_uuid(&claim.job_id)
        || !matches!(claim.state.as_str(), "leased" | "running")
        || !(1..=64).contains(&claim.attempt)
        || !(1..=MAX_SAFE_INTEGER).contains(&claim.revision)
        || claim.lease_expires_at_ms > MAX_SAFE_INTEGER
        || claim.lease_expires_at_ms <= current_time_ms()
        || !profile.source_count().contains(&sources.len())
        || outputs.len() != 1
    {
        bail!("control plane returned an invalid native job claim");
    }
    if plural {
        if claim.source.is_some()
            || claim.output.is_some()
            || claim.sources.is_empty()
            || claim.outputs.is_empty()
            || claim.native_plan_schema_version != Some(NATIVE_EXECUTION_PLAN_SCHEMA_VERSION)
            || claim.media_job_catalog_version != Some(MEDIA_JOB_CATALOG_VERSION)
            || claim.media_service_catalog_version != Some(MEDIA_SERVICE_CATALOG_VERSION)
            || claim.execution_origin != Some(profile.expected_origin())
            || claim.sandbox != Some(profile.sandbox()?)
        {
            bail!("control plane returned an invalid versioned native job claim");
        }
    } else if profile != NativeProfile::Frame
        || claim.execution_origin.is_some()
        || claim.native_plan_schema_version.is_some()
        || claim.media_job_catalog_version.is_some()
        || claim.media_service_catalog_version.is_some()
        || claim.sandbox.is_some()
        || outputs[0].content_type != "image/png"
    {
        bail!("control plane returned an invalid legacy native job claim");
    }
    let mut total_source_bytes = 0_u64;
    for (index, source) in sources.iter().enumerate() {
        if source.bytes == 0
            || source.bytes > spec.sandbox.max_source_bytes
            || source.bytes > MAX_SOURCE_BYTES
            || !valid_sha256(&source.checksum_sha256)
            || !valid_source_content_type(&source.content_type)
            || (plural && source.ordinal != u16::try_from(index).ok())
            || (!plural && source.ordinal.is_some())
        {
            bail!("control plane returned an invalid native source descriptor");
        }
        total_source_bytes = total_source_bytes
            .checked_add(source.bytes)
            .ok_or_else(|| anyhow::anyhow!("native source manifest overflowed"))?;
        validate_descriptor_path(&claim.job_id, &source.path, "sources", index, plural)?;
    }
    if total_source_bytes > spec.sandbox.max_source_bytes {
        bail!("control plane returned an oversized native source set");
    }
    let output = &outputs[0];
    if output.max_bytes == 0
        || output.max_bytes > spec.sandbox.max_output_bytes
        || output.max_bytes > MAX_OUTPUT_BYTES
        || !spec
            .output_content_types
            .contains(&output.content_type.as_str())
        || (plural
            && (output.ordinal != Some(0) || output.role.as_deref() != Some(profile.output_role())))
        || (!plural && (output.ordinal.is_some() || output.role.is_some()))
    {
        bail!("control plane returned an invalid native output descriptor");
    }
    validate_descriptor_path(&claim.job_id, &output.path, "outputs", 0, plural)?;
    validate_job_path(&claim.job_id, &claim.heartbeat_path, "heartbeat")?;
    validate_job_path(&claim.job_id, &claim.progress_path, "progress")?;
    validate_job_path(&claim.job_id, &claim.complete_path, "complete")?;
    validate_job_path(&claim.job_id, &claim.fail_path, "fail")?;
    Ok(())
}

fn validate_descriptor_path(
    job_id: &str,
    path: &str,
    plural_suffix: &str,
    ordinal: usize,
    plural: bool,
) -> Result<()> {
    let expected = if plural {
        format!("/api/v1/worker/media-jobs/{job_id}/{plural_suffix}/{ordinal}")
    } else {
        let singular = match plural_suffix {
            "sources" => "source",
            "outputs" => "output",
            _ => bail!("worker descriptor suffix is invalid"),
        };
        format!("/api/v1/worker/media-jobs/{job_id}/{singular}")
    };
    if path != expected || !safe_private_path(path) {
        bail!("control plane returned an invalid worker descriptor path");
    }
    Ok(())
}

fn valid_source_content_type(value: &str) -> bool {
    matches!(
        value,
        "video/mp4"
            | "video/quicktime"
            | "video/webm"
            | "video/x-matroska"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/wav"
            | "audio/webm"
            | "audio/ogg"
            | "application/json"
    )
}

fn validate_job_state(state: &WorkerJobResponse, expected_job_id: &str) -> Result<()> {
    if state.schema_version != API_SCHEMA_VERSION
        || state.job_id != expected_job_id
        || !matches!(
            state.state.as_str(),
            "queued" | "leased" | "running" | "succeeded" | "failed" | "cancelled"
        )
        || state.attempt == 0
        || !(1..=MAX_SAFE_INTEGER).contains(&state.revision)
        || state
            .lease_expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms > MAX_SAFE_INTEGER)
        || state
            .progress_basis_points
            .is_some_and(|value| value > 10_000)
        || (matches!(state.state.as_str(), "leased" | "running")
            && state
                .lease_expires_at_ms
                .is_none_or(|expires_at_ms| expires_at_ms <= current_time_ms()))
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

#[cfg(test)]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn sha256_file(
    path: &Path,
    max_bytes: u64,
    cancellation: &CancellationToken,
) -> Result<(u64, String)> {
    if cancellation.is_cancelled() {
        return Err(ProtocolApiError::new(
            "cancellation_requested",
            ApiFailureDisposition::Cancelled,
        )
        .into());
    }
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| anyhow::anyhow!("native output file could not be opened"))?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1_024];
    loop {
        if cancellation.is_cancelled() {
            return Err(ProtocolApiError::new(
                "cancellation_requested",
                ApiFailureDisposition::Cancelled,
            )
            .into());
        }
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|_| anyhow::anyhow!("native output file could not be read"))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .filter(|value| *value <= max_bytes && *value <= MAX_OUTPUT_BYTES)
            .ok_or_else(|| anyhow::anyhow!("native output exceeded its immutable manifest"))?;
        digest.update(&buffer[..read]);
    }
    let checksum = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((total, checksum))
}

fn new_idempotency_key() -> String {
    format!("worker:{}", Uuid::now_v7())
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(u64::MAX)
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
        | "service_unavailable"
        | "source_not_ready"
        | "storage_unavailable"
        | "upload_reconciliation_required" => ApiFailureDisposition::Retryable,
        "invalid_output_manifest"
        | "invalid_probe_manifest"
        | "invalid_probe_source"
        | "output_acknowledgement_invalid"
        | "output_conflict"
        | "output_invalid"
        | "profile_unavailable"
        | "source_invalid"
        | "source_manifest_invalid"
        | "unsupported_media_type"
        | "unsupported_source_media_type" => ApiFailureDisposition::Permanent,
        "cancellation_requested" => ApiFailureDisposition::Cancelled,
        "lease_conflict" | "revision_conflict" => ApiFailureDisposition::LeaseLost,
        _ => ApiFailureDisposition::Configuration,
    }
}

async fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T> {
    reject_content_encoding(&response)?;
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
            classify_api_code("invalid_probe_manifest"),
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
            classify_api_code("revision_conflict"),
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
        assert!(!debug.contains(TENANT));
        let lease = LeaseToken::generate();
        let raw = lease.as_str().to_owned();
        assert_eq!(raw.len(), 64);
        assert!(raw.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!format!("{lease:?}").contains(&raw));
        let source = WorkerSourceDescriptor {
            ordinal: Some(0),
            path: format!("/api/v1/worker/media-jobs/{JOB}/sources/0"),
            bytes: 12,
            checksum_sha256: "a".repeat(64),
            content_type: "video/webm".into(),
        };
        let source_debug = format!("{source:?}");
        assert!(source_debug.contains("<redacted>"));
        assert!(!source_debug.contains(JOB));
        assert!(!source_debug.contains(&"a".repeat(64)));
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
            native_plan_schema_version: None,
            media_job_catalog_version: None,
            media_service_catalog_version: None,
            job_id: JOB.into(),
            state: "leased".into(),
            profile: "thumbnail_v1".into(),
            execution_origin: None,
            sandbox: None,
            attempt: 1,
            revision: 2,
            lease_expires_at_ms: current_time_ms() + 60_000,
            source: Some(WorkerSourceDescriptor {
                ordinal: None,
                path: format!("/api/v1/worker/media-jobs/{JOB}/source"),
                bytes: 12,
                checksum_sha256: "a".repeat(64),
                content_type: "video/webm".into(),
            }),
            sources: Vec::new(),
            output: Some(WorkerOutputDescriptor {
                ordinal: None,
                role: None,
                path: format!("/api/v1/worker/media-jobs/{JOB}/output"),
                content_type: "image/png".into(),
                max_bytes: 1_024,
            }),
            outputs: Vec::new(),
            heartbeat_path: format!("/api/v1/worker/media-jobs/{JOB}/heartbeat"),
            progress_path: format!("/api/v1/worker/media-jobs/{JOB}/progress"),
            complete_path: format!("/api/v1/worker/media-jobs/{JOB}/complete"),
            fail_path: format!("/api/v1/worker/media-jobs/{JOB}/fail"),
        };
        assert!(validate_claim(&claim).is_ok());
        let Some(source) = claim.source.as_mut() else {
            panic!("legacy source descriptor missing");
        };
        source.bytes = MAX_SOURCE_BYTES + 1;
        assert!(validate_claim(&claim).is_err());
    }

    #[test]
    fn active_job_responses_require_a_live_lease() {
        let mut state = WorkerJobResponse {
            schema_version: API_SCHEMA_VERSION,
            job_id: JOB.into(),
            state: "running".into(),
            attempt: 1,
            revision: 2,
            progress_basis_points: Some(1_000),
            cancel_requested: false,
            lease_expires_at_ms: Some(current_time_ms() + 60_000),
            retry_scheduled: false,
        };
        assert!(validate_job_state(&state, JOB).is_ok());
        state.lease_expires_at_ms = Some(current_time_ms().saturating_sub(1));
        assert!(validate_job_state(&state, JOB).is_err());
        state.state = "succeeded".into();
        state.progress_basis_points = Some(10_000);
        state.lease_expires_at_ms = None;
        assert!(validate_job_state(&state, JOB).is_ok());
    }

    #[test]
    fn plural_claim_contract_covers_every_retained_native_profile() {
        for profile in NativeProfile::ALL {
            let source_count = if profile == NativeProfile::SegmentMux {
                2
            } else {
                1
            };
            let spec = media_service_catalog()
                .get(profile.job_kind())
                .expect("native catalog row");
            let claim = NativeJobClaimResponse {
                schema_version: API_SCHEMA_VERSION,
                native_plan_schema_version: Some(NATIVE_EXECUTION_PLAN_SCHEMA_VERSION),
                media_job_catalog_version: Some(MEDIA_JOB_CATALOG_VERSION),
                media_service_catalog_version: Some(MEDIA_SERVICE_CATALOG_VERSION),
                job_id: JOB.into(),
                state: "leased".into(),
                profile: profile.profile_id().into(),
                execution_origin: Some(profile.expected_origin()),
                sandbox: Some(profile.sandbox().expect("native sandbox")),
                attempt: 1,
                revision: 2,
                lease_expires_at_ms: current_time_ms() + 60_000,
                source: None,
                sources: (0..source_count)
                    .map(|ordinal| WorkerSourceDescriptor {
                        ordinal: Some(ordinal),
                        path: format!("/api/v1/worker/media-jobs/{JOB}/sources/{ordinal}"),
                        bytes: 12,
                        checksum_sha256: "a".repeat(64),
                        content_type: "video/webm".into(),
                    })
                    .collect(),
                output: None,
                outputs: vec![WorkerOutputDescriptor {
                    ordinal: Some(0),
                    role: Some(profile.output_role().into()),
                    path: format!("/api/v1/worker/media-jobs/{JOB}/outputs/0"),
                    content_type: spec.output_content_types[0].into(),
                    max_bytes: spec.sandbox.max_output_bytes,
                }],
                heartbeat_path: format!("/api/v1/worker/media-jobs/{JOB}/heartbeat"),
                progress_path: format!("/api/v1/worker/media-jobs/{JOB}/progress"),
                complete_path: format!("/api/v1/worker/media-jobs/{JOB}/complete"),
                fail_path: format!("/api/v1/worker/media-jobs/{JOB}/fail"),
            };
            assert!(validate_claim(&claim).is_ok(), "{}", profile.profile_id());
        }
    }

    #[test]
    fn plural_claim_rejects_sparse_sources_and_multi_source_underflow() {
        let profile = NativeProfile::SegmentMux;
        let spec = media_service_catalog()
            .get(profile.job_kind())
            .expect("segment mux catalog row");
        let mut claim = NativeJobClaimResponse {
            schema_version: API_SCHEMA_VERSION,
            native_plan_schema_version: Some(NATIVE_EXECUTION_PLAN_SCHEMA_VERSION),
            media_job_catalog_version: Some(MEDIA_JOB_CATALOG_VERSION),
            media_service_catalog_version: Some(MEDIA_SERVICE_CATALOG_VERSION),
            job_id: JOB.into(),
            state: "leased".into(),
            profile: profile.profile_id().into(),
            execution_origin: Some(profile.expected_origin()),
            sandbox: Some(profile.sandbox().expect("native sandbox")),
            attempt: 1,
            revision: 2,
            lease_expires_at_ms: current_time_ms() + 60_000,
            source: None,
            sources: vec![WorkerSourceDescriptor {
                ordinal: Some(0),
                path: format!("/api/v1/worker/media-jobs/{JOB}/sources/0"),
                bytes: 12,
                checksum_sha256: "a".repeat(64),
                content_type: "video/webm".into(),
            }],
            output: None,
            outputs: vec![WorkerOutputDescriptor {
                ordinal: Some(0),
                role: Some(profile.output_role().into()),
                path: format!("/api/v1/worker/media-jobs/{JOB}/outputs/0"),
                content_type: spec.output_content_types[0].into(),
                max_bytes: spec.sandbox.max_output_bytes,
            }],
            heartbeat_path: format!("/api/v1/worker/media-jobs/{JOB}/heartbeat"),
            progress_path: format!("/api/v1/worker/media-jobs/{JOB}/progress"),
            complete_path: format!("/api/v1/worker/media-jobs/{JOB}/complete"),
            fail_path: format!("/api/v1/worker/media-jobs/{JOB}/fail"),
        };
        assert!(validate_claim(&claim).is_err());
        claim.sources.push(WorkerSourceDescriptor {
            ordinal: Some(2),
            path: format!("/api/v1/worker/media-jobs/{JOB}/sources/1"),
            bytes: 12,
            checksum_sha256: "b".repeat(64),
            content_type: "video/webm".into(),
        });
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
