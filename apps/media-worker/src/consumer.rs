use std::{fmt, time::Duration};

use anyhow::Result;
use frame_media::CancellationToken;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    protocol::{ApiFailureDisposition, ClaimedJob, ProtocolApiError, WorkerClient, sha256_hex},
    thumbnail::{ThumbnailError, ensure_thumbnail_runtime, render_thumbnail_v1},
};

const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const ERROR_BACKOFF: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const INITIAL_PROGRESS: u16 = 1_000;
const OUTPUT_READY_PROGRESS: u16 = 9_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkOutcome {
    Idle,
    Completed { job_id: String },
}

#[derive(Debug, Clone, Copy)]
struct JobFailure {
    error_class: &'static str,
    retryable: bool,
    report_failure: bool,
    halt_consumer: bool,
    message: &'static str,
}

impl JobFailure {
    const fn transport(message: &'static str) -> Self {
        Self {
            error_class: "transport_failure",
            retryable: true,
            report_failure: true,
            halt_consumer: false,
            message,
        }
    }

    const fn cancelled() -> Self {
        Self {
            error_class: "cancelled",
            retryable: false,
            report_failure: false,
            halt_consumer: false,
            message: "the native media job was cancelled",
        }
    }

    const fn output(message: &'static str) -> Self {
        Self {
            error_class: "output_invalid",
            retryable: false,
            report_failure: true,
            halt_consumer: false,
            message,
        }
    }

    fn from_protocol(error: anyhow::Error, message: &'static str) -> Self {
        let Some(protocol) = error.downcast_ref::<ProtocolApiError>() else {
            return Self::transport(message);
        };
        match protocol.disposition() {
            ApiFailureDisposition::Retryable => Self::transport(message),
            ApiFailureDisposition::Permanent => Self {
                error_class: if protocol.code().contains("source")
                    || protocol.code().contains("media_type")
                {
                    "input_invalid"
                } else {
                    "output_invalid"
                },
                retryable: false,
                report_failure: true,
                halt_consumer: false,
                message,
            },
            ApiFailureDisposition::Cancelled => Self::cancelled(),
            ApiFailureDisposition::LeaseLost | ApiFailureDisposition::Configuration => Self {
                error_class: "transport_failure",
                retryable: false,
                report_failure: false,
                halt_consumer: protocol.disposition() == ApiFailureDisposition::Configuration,
                message,
            },
        }
    }

    const fn from_thumbnail(error: ThumbnailError) -> Self {
        Self {
            error_class: error.error_class(),
            retryable: error.retryable(),
            report_failure: !matches!(error, ThumbnailError::Cancelled),
            halt_consumer: false,
            message: match error {
                ThumbnailError::InvalidInput => "the native media input is invalid",
                ThumbnailError::MissingRuntime => "the native media runtime is unavailable",
                ThumbnailError::Pipeline => "the native media pipeline failed",
                ThumbnailError::Timeout => "the native media pipeline timed out",
                ThumbnailError::Cancelled => "the native media pipeline was cancelled",
                ThumbnailError::ResourceLimit => {
                    "the native media operation exceeded a resource limit"
                }
                ThumbnailError::InvalidOutput => "the native media output is invalid",
            },
        }
    }
}

impl fmt::Display for JobFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for JobFailure {}

struct PipelineHeartbeatGuard {
    cancellation: CancellationToken,
    stop: watch::Sender<bool>,
    heartbeat: Option<tokio::task::JoinHandle<std::result::Result<(), JobFailure>>>,
    active: bool,
}

impl PipelineHeartbeatGuard {
    fn new(
        cancellation: CancellationToken,
        stop: watch::Sender<bool>,
        heartbeat: tokio::task::JoinHandle<std::result::Result<(), JobFailure>>,
    ) -> Self {
        Self {
            cancellation,
            stop,
            heartbeat: Some(heartbeat),
            active: true,
        }
    }

    fn request_stop(&self) {
        let _ = self.stop.send(true);
    }

    fn heartbeat_mut(
        &mut self,
    ) -> std::result::Result<
        &mut tokio::task::JoinHandle<std::result::Result<(), JobFailure>>,
        JobFailure,
    > {
        self.heartbeat
            .as_mut()
            .ok_or_else(|| JobFailure::transport("native media heartbeat task was unavailable"))
    }

    fn disarm(&mut self) {
        self.active = false;
        self.heartbeat.take();
    }
}

impl Drop for PipelineHeartbeatGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.cancellation.cancel();
        self.request_stop();
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
    }
}

pub(crate) async fn work_once(client: &WorkerClient) -> Result<WorkOutcome> {
    ensure_thumbnail_runtime()?;
    let Some(job) = client.claim().await? else {
        return Ok(WorkOutcome::Idle);
    };
    info!(
        job_id = %job.claim.job_id,
        profile = %job.claim.profile,
        "native media job claimed"
    );

    match process_claim(client, &job).await {
        Ok(()) => {
            info!(job_id = %job.claim.job_id, "native media job completed");
            Ok(WorkOutcome::Completed {
                job_id: job.claim.job_id.clone(),
            })
        }
        Err(failure) => {
            if failure.report_failure
                && client
                    .fail(&job, failure.error_class, failure.retryable)
                    .await
                    .is_err()
            {
                warn!(
                    job_id = %job.claim.job_id,
                    error_class = failure.error_class,
                    "native media failure callback was not acknowledged"
                );
            }
            Err(failure.into())
        }
    }
}

async fn process_claim(
    client: &WorkerClient,
    job: &ClaimedJob,
) -> std::result::Result<(), JobFailure> {
    let state = client
        .heartbeat(job)
        .await
        .map_err(|error| JobFailure::from_protocol(error, "native media heartbeat failed"))?;
    ensure_not_cancelled(&state)?;

    let source = client.download_source(job).await.map_err(|error| {
        JobFailure::from_protocol(error, "native media source transport failed")
    })?;
    let state = client
        .progress(job, INITIAL_PROGRESS)
        .await
        .map_err(|error| {
            JobFailure::from_protocol(error, "native media progress callback failed")
        })?;
    ensure_not_cancelled(&state)?;

    let cancellation = CancellationToken::new();
    let (stop_heartbeats, heartbeat_stop) = watch::channel(false);
    let heartbeat_task = tokio::spawn(heartbeat_loop(
        client.clone(),
        job.clone(),
        cancellation.clone(),
        heartbeat_stop,
    ));
    let background =
        PipelineHeartbeatGuard::new(cancellation.clone(), stop_heartbeats, heartbeat_task);
    let output_limit = job.claim.output.max_bytes;
    let pipeline_cancellation = cancellation.clone();
    let pipeline_task = tokio::task::spawn_blocking(move || {
        render_thumbnail_v1(&source, output_limit, &pipeline_cancellation)
    });
    let output = finish_pipeline(pipeline_task, background).await?;
    if cancellation.is_cancelled() {
        return Err(JobFailure::cancelled());
    }

    let state = client
        .progress(job, OUTPUT_READY_PROGRESS)
        .await
        .map_err(|error| {
            JobFailure::from_protocol(error, "native media progress callback failed")
        })?;
    ensure_not_cancelled(&state)?;
    let bytes = output.len() as u64;
    let checksum = sha256_hex(&output);
    let acknowledgement = client
        .upload_output(job, output, &checksum)
        .await
        .map_err(|error| {
            JobFailure::from_protocol(error, "native media output transport failed")
        })?;
    if acknowledgement.bytes != bytes {
        return Err(JobFailure::output(
            "native media output acknowledgement was inconsistent",
        ));
    }
    let completed = client
        .complete(job, bytes, &checksum)
        .await
        .map_err(|error| {
            JobFailure::from_protocol(error, "native media completion callback failed")
        })?;
    if completed.cancel_requested || completed.state != "succeeded" {
        return Err(
            if completed.cancel_requested || completed.state == "cancelled" {
                JobFailure::cancelled()
            } else {
                JobFailure::output(
                    "control plane did not accept the native media output as terminal",
                )
            },
        );
    }
    Ok(())
}

async fn finish_pipeline(
    pipeline_task: tokio::task::JoinHandle<std::result::Result<Vec<u8>, ThumbnailError>>,
    mut background: PipelineHeartbeatGuard,
) -> std::result::Result<Vec<u8>, JobFailure> {
    let pipeline_result = pipeline_task.await;
    background.request_stop();
    let heartbeat_result = background.heartbeat_mut()?.await;
    background.disarm();
    let pipeline_result =
        pipeline_result.map_err(|_| JobFailure::transport("native media pipeline task failed"))?;
    let heartbeat_result = heartbeat_result
        .map_err(|_| JobFailure::transport("native media heartbeat task failed"))?;
    heartbeat_result?;
    pipeline_result.map_err(JobFailure::from_thumbnail)
}

async fn heartbeat_loop(
    client: WorkerClient,
    job: ClaimedJob,
    cancellation: CancellationToken,
    mut stop: watch::Receiver<bool>,
) -> std::result::Result<(), JobFailure> {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() {
                    cancellation.cancel();
                    return Ok(());
                }
                if *stop.borrow() {
                    return Ok(());
                }
            }
            _ = interval.tick() => {
                match client.heartbeat(&job).await {
                    Ok(state) if state.cancel_requested || state.state == "cancelled" => {
                        cancellation.cancel();
                        return Err(JobFailure::cancelled());
                    }
                    Ok(_) => {}
                    Err(error) => {
                        cancellation.cancel();
                        return Err(JobFailure::from_protocol(
                            error,
                            "native media heartbeat failed",
                        ));
                    }
                }
            }
        }
    }
}

fn ensure_not_cancelled(
    state: &crate::protocol::WorkerJobResponse,
) -> std::result::Result<(), JobFailure> {
    if state.cancel_requested || state.state == "cancelled" {
        Err(JobFailure::cancelled())
    } else {
        Ok(())
    }
}

pub(crate) async fn run_consumer_loop(client: WorkerClient, mut shutdown: watch::Receiver<bool>) {
    loop {
        let attempt = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
                continue;
            }
            result = work_once(&client) => result,
        };

        let delay = match attempt {
            Ok(WorkOutcome::Idle) => IDLE_POLL_INTERVAL,
            Ok(WorkOutcome::Completed { .. }) => Duration::ZERO,
            Err(error)
                if error
                    .downcast_ref::<JobFailure>()
                    .is_some_and(|failure| failure.halt_consumer)
                    || error
                        .downcast_ref::<ProtocolApiError>()
                        .is_some_and(|failure| {
                            failure.disposition() == ApiFailureDisposition::Configuration
                        }) =>
            {
                warn!("native media consumer stopped after a protocol configuration failure");
                break;
            }
            Err(_) => {
                warn!("native media consumer attempt failed; retrying after bounded backoff");
                ERROR_BACKOFF
            }
        };
        if delay.is_zero() {
            continue;
        }
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(delay) => {}
        }
    }
    info!("native media consumer stopped");
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::{get, post, put},
    };
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::*;
    use crate::protocol::{WorkerConfig, sha256_hex};

    const TENANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f123";
    const JOB: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f124";
    const TOKEN: &str = "frame-worker-test-secret-000000001";

    #[derive(Clone)]
    struct MockControlPlane {
        source: Arc<Vec<u8>>,
        source_checksum: Arc<String>,
        lease: Arc<Mutex<Option<String>>>,
        output: Arc<Mutex<Option<Vec<u8>>>>,
        fail_calls: Arc<AtomicUsize>,
    }

    impl MockControlPlane {
        fn assert_headers(&self, headers: &HeaderMap) {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer frame-worker-test-secret-000000001")
            );
            assert_eq!(
                headers
                    .get("x-frame-tenant-id")
                    .and_then(|value| value.to_str().ok()),
                Some(TENANT)
            );
            assert_eq!(
                headers
                    .get(header::ACCEPT_ENCODING)
                    .and_then(|value| value.to_str().ok()),
                Some("identity")
            );
            assert!(
                headers.contains_key("idempotency-key") || headers.contains_key(header::ACCEPT)
            );
        }

        fn assert_lease(&self, headers: &HeaderMap) {
            let actual = headers
                .get("x-frame-lease-token")
                .and_then(|value| value.to_str().ok())
                .expect("lease header");
            let lease = self.lease.lock().expect("lease lock");
            assert_eq!(lease.as_deref(), Some(actual));
        }
    }

    async fn claim_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.assert_headers(&headers);
        assert!(headers.contains_key("idempotency-key"));
        let lease = headers
            .get("x-frame-lease-token")
            .and_then(|value| value.to_str().ok())
            .expect("claim lease");
        assert_eq!(lease.len(), 64);
        assert!(
            lease
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        );
        *state.lease.lock().expect("lease lock") = Some(lease.into());
        assert_eq!(body, json!({"schema_version": 1, "tenant_id": TENANT}));
        Json(json!({
            "schema_version": 1,
            "job_id": JOB,
            "state": "leased",
            "profile": "thumbnail_v1",
            "attempt": 1,
            "revision": 2,
            "lease_expires_at_ms": test_now_ms() + 60_000,
            "source": {
                "path": format!("/api/v1/worker/media-jobs/{JOB}/source"),
                "bytes": state.source.len(),
                "checksum_sha256": state.source_checksum.as_str(),
                "content_type": "video/webm"
            },
            "output": {
                "path": format!("/api/v1/worker/media-jobs/{JOB}/output"),
                "content_type": "image/png",
                "max_bytes": 32 * 1_024 * 1_024
            },
            "heartbeat_path": format!("/api/v1/worker/media-jobs/{JOB}/heartbeat"),
            "progress_path": format!("/api/v1/worker/media-jobs/{JOB}/progress"),
            "complete_path": format!("/api/v1/worker/media-jobs/{JOB}/complete"),
            "fail_path": format!("/api/v1/worker/media-jobs/{JOB}/fail")
        }))
    }

    async fn source_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        state.assert_headers(&headers);
        state.assert_lease(&headers);
        assert_eq!(
            headers
                .get(header::ACCEPT)
                .and_then(|value| value.to_str().ok()),
            Some("video/webm")
        );
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE.as_str(), "video/webm".into()),
                (
                    header::CONTENT_LENGTH.as_str(),
                    state.source.len().to_string(),
                ),
                (
                    "x-content-sha256",
                    state.source_checksum.as_str().to_owned(),
                ),
            ],
            state.source.as_ref().clone(),
        )
    }

    async fn heartbeat_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.assert_headers(&headers);
        state.assert_lease(&headers);
        assert!(headers.contains_key("idempotency-key"));
        assert_eq!(body, json!({"schema_version": 1, "tenant_id": TENANT}));
        Json(running_state(None))
    }

    async fn progress_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.assert_headers(&headers);
        state.assert_lease(&headers);
        assert!(headers.contains_key("idempotency-key"));
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["tenant_id"], TENANT);
        let progress = body["progress_basis_points"].as_u64().expect("progress") as u16;
        assert!(matches!(progress, INITIAL_PROGRESS | OUTPUT_READY_PROGRESS));
        Json(running_state(Some(progress)))
    }

    async fn output_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Json<Value> {
        state.assert_headers(&headers);
        state.assert_lease(&headers);
        assert!(headers.contains_key("idempotency-key"));
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/png")
        );
        assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"));
        let checksum = sha256_hex(&body);
        assert_eq!(
            headers
                .get("x-content-sha256")
                .and_then(|value| value.to_str().ok()),
            Some(checksum.as_str())
        );
        *state.output.lock().expect("output lock") = Some(body.to_vec());
        Json(json!({
            "schema_version": 1,
            "job_id": JOB,
            "accepted": true,
            "bytes": body.len(),
            "checksum_sha256": checksum,
            "content_type": "image/png"
        }))
    }

    async fn complete_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.assert_headers(&headers);
        state.assert_lease(&headers);
        assert!(headers.contains_key("idempotency-key"));
        let output = state.output.lock().expect("output lock");
        let output = output.as_deref().expect("uploaded output");
        assert_eq!(body["bytes"], output.len());
        assert_eq!(body["checksum_sha256"], sha256_hex(output));
        assert_eq!(body["content_type"], "image/png");
        Json(json!({
            "schema_version": 1,
            "job_id": JOB,
            "state": "succeeded",
            "attempt": 1,
            "revision": 5,
            "progress_basis_points": 10000,
            "cancel_requested": false,
            "lease_expires_at_ms": null,
            "retry_scheduled": false
        }))
    }

    async fn fail_handler(
        State(state): State<MockControlPlane>,
        headers: HeaderMap,
        Json(_body): Json<Value>,
    ) -> Json<Value> {
        state.assert_headers(&headers);
        state.assert_lease(&headers);
        state.fail_calls.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "schema_version": 1,
            "job_id": JOB,
            "state": "failed",
            "attempt": 1,
            "revision": 5,
            "progress_basis_points": 1000,
            "cancel_requested": false,
            "lease_expires_at_ms": null,
            "retry_scheduled": false
        }))
    }

    fn running_state(progress: Option<u16>) -> Value {
        json!({
            "schema_version": 1,
            "job_id": JOB,
            "state": "running",
            "attempt": 1,
            "revision": 3,
            "progress_basis_points": progress,
            "cancel_requested": false,
            "lease_expires_at_ms": test_now_ms() + 60_000,
            "retry_scheduled": false
        })
    }

    fn test_now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64
    }

    #[tokio::test]
    async fn local_protocol_walking_slice_needs_no_provider_credentials() {
        let fixture = std::env::temp_dir().join(format!(
            "frame-media-worker-protocol-{}.webm",
            Uuid::now_v7().simple()
        ));
        frame_media::record_synthetic_webm(&fixture).expect("synthetic fixture");
        let source = std::fs::read(&fixture).expect("source fixture");
        let _ = std::fs::remove_file(&fixture);
        let state = MockControlPlane {
            source_checksum: Arc::new(sha256_hex(&source)),
            source: Arc::new(source),
            lease: Arc::new(Mutex::new(None)),
            output: Arc::new(Mutex::new(None)),
            fail_calls: Arc::new(AtomicUsize::new(0)),
        };
        let job_root = format!("/api/v1/worker/media-jobs/{JOB}");
        let app = Router::new()
            .route("/api/v1/worker/media-jobs/claim", post(claim_handler))
            .route(&format!("{job_root}/source"), get(source_handler))
            .route(&format!("{job_root}/heartbeat"), post(heartbeat_handler))
            .route(&format!("{job_root}/progress"), post(progress_handler))
            .route(&format!("{job_root}/output"), put(output_handler))
            .route(&format!("{job_root}/complete"), post(complete_handler))
            .route(&format!("{job_root}/fail"), post(fail_handler))
            .with_state(state.clone());
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                // Some hermetic test sandboxes prohibit even loopback sockets. The same test
                // executes the full walking slice anywhere loopback networking is available.
                return;
            }
            Err(error) => panic!("mock listener failed: {error}"),
        };
        let address = listener.local_addr().expect("mock address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock server");
        });
        let config = WorkerConfig::local_for_test(&format!("http://{address}"), TOKEN, TENANT)
            .expect("worker config");
        let client = WorkerClient::new(config).expect("worker client");
        let outcome = work_once(&client).await.expect("walking slice");
        assert_eq!(outcome, WorkOutcome::Completed { job_id: JOB.into() });
        {
            let output = state.output.lock().expect("output lock");
            let output = output.as_deref().expect("uploaded output");
            assert!(output.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
        assert_eq!(state.fail_calls.load(Ordering::SeqCst), 0);
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn failure_classes_are_allowlisted_and_privacy_safe() {
        for error in [
            ThumbnailError::InvalidInput,
            ThumbnailError::MissingRuntime,
            ThumbnailError::Pipeline,
            ThumbnailError::Timeout,
            ThumbnailError::Cancelled,
            ThumbnailError::ResourceLimit,
            ThumbnailError::InvalidOutput,
        ] {
            let failure = JobFailure::from_thumbnail(error);
            assert!(matches!(
                failure.error_class,
                "input_invalid"
                    | "pipeline_failure"
                    | "pipeline_timeout"
                    | "cancelled"
                    | "resource_limit"
                    | "output_invalid"
            ));
            assert!(!failure.to_string().contains('/'));
            assert!(!failure.to_string().contains("token"));
        }
    }

    #[test]
    fn transient_failures_are_retryable_but_cancellation_is_not() {
        assert!(JobFailure::transport("transport failed").retryable);
        assert!(!JobFailure::cancelled().retryable);
        assert!(JobFailure::from_thumbnail(ThumbnailError::Timeout).retryable);
        assert!(!JobFailure::from_thumbnail(ThumbnailError::Pipeline).retryable);
    }

    #[test]
    fn protocol_dispositions_prevent_retry_burn_and_unsafe_callbacks() {
        let permanent = JobFailure::from_protocol(
            ProtocolApiError::new("output_conflict", ApiFailureDisposition::Permanent).into(),
            "output rejected",
        );
        assert!(!permanent.retryable);
        assert!(permanent.report_failure);
        assert_eq!(permanent.error_class, "output_invalid");

        let lease_lost = JobFailure::from_protocol(
            ProtocolApiError::new("lease_conflict", ApiFailureDisposition::LeaseLost).into(),
            "lease lost",
        );
        assert!(!lease_lost.retryable);
        assert!(!lease_lost.report_failure);

        let configuration = JobFailure::from_protocol(
            ProtocolApiError::new(
                "unrecognized_api_error",
                ApiFailureDisposition::Configuration,
            )
            .into(),
            "configuration failed",
        );
        assert!(!configuration.retryable);
        assert!(!configuration.report_failure);
        assert!(configuration.halt_consumer);
    }

    #[tokio::test]
    async fn pipeline_join_failure_always_stops_and_joins_heartbeats() {
        let (stop, mut stopped) = watch::channel(false);
        let observed = Arc::new(AtomicUsize::new(0));
        let heartbeat_observed = Arc::clone(&observed);
        let heartbeat = tokio::spawn(async move {
            let _ = stopped.changed().await;
            if *stopped.borrow() {
                heartbeat_observed.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        });
        let pipeline =
            tokio::task::spawn_blocking(|| -> std::result::Result<Vec<u8>, ThumbnailError> {
                panic!("synthetic pipeline task panic")
            });
        let background = PipelineHeartbeatGuard::new(CancellationToken::new(), stop, heartbeat);
        let error = finish_pipeline(pipeline, background)
            .await
            .expect_err("join failure");
        assert_eq!(error.error_class, "transport_failure");
        assert_eq!(observed.load(Ordering::SeqCst), 1);
    }
}
