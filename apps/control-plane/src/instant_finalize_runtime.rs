//! D1 authority for the Instant multipart-to-publication boundary.

use crate::instant_finalize_contract::{
    INSTANT_FINALIZE_SCHEMA_VERSION, InstantFinalizeReceiptV1, InstantFinalizeRequestV1,
    InstantFinalizeStateV1,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement};

const MAX_RECONCILE_ATTEMPTS: i64 = 8;
const RETRY_BASE_MS: i64 = 1_000;
const RETRY_MAX_MS: i64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstantFinalizeFailure {
    Conflict,
    Pending,
    Persistence,
}

impl InstantFinalizeFailure {
    pub(crate) const fn safe_code(self) -> &'static str {
        match self {
            Self::Conflict => "instant_finalize_conflict",
            Self::Pending => "instant_finalize_pending",
            Self::Persistence => "instant_finalize_unavailable",
        }
    }

    const fn retained_class(self) -> &'static str {
        match self {
            Self::Conflict => "conflict",
            Self::Pending => "dependency_pending",
            Self::Persistence => "persistence",
        }
    }
}

#[derive(Debug, Deserialize)]
struct RequestRow {
    session_id: String,
    organization_id: String,
    upload_id: String,
    video_id: String,
    ordered_parts_sha256: String,
    object_version: String,
    job_id: String,
    job_generation: i64,
    request_sha256: String,
    state: String,
    publication_id: Option<String>,
    playable_object_key: Option<String>,
    distribution_eligible: i64,
    reconcile_attempt_count: i64,
    last_failure_class: Option<String>,
}

impl RequestRow {
    fn exact(&self, request: &InstantFinalizeRequestV1) -> bool {
        self.session_id == request.session_id
            && self.organization_id == request.tenant_id
            && self.upload_id == request.upload_id
            && self.video_id == request.video_id
            && self.ordered_parts_sha256 == request.ordered_parts_sha256
            && self.object_version == request.object_version
            && self.job_id == request.job_id
            && self.job_generation == request.job_generation as i64
            && self.request_sha256 == request.request_sha256
    }

    fn receipt(&self) -> Result<InstantFinalizeReceiptV1, InstantFinalizeFailure> {
        let state = match self.state.as_str() {
            "pending" => InstantFinalizeStateV1::Pending,
            "published" => InstantFinalizeStateV1::Published,
            _ => return Err(InstantFinalizeFailure::Conflict),
        };
        Ok(InstantFinalizeReceiptV1 {
            schema_version: INSTANT_FINALIZE_SCHEMA_VERSION,
            state,
            request_sha256: self.request_sha256.clone(),
            publication_id: self.publication_id.clone(),
            job_id: self.job_id.clone(),
            job_generation: u64::try_from(self.job_generation)
                .map_err(|_| InstantFinalizeFailure::Persistence)?,
            upload_id: self.upload_id.clone(),
            object_version: self.object_version.clone(),
            playable_object_key: self.playable_object_key.clone(),
            distribution_eligible: self.distribution_eligible == 1,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ReservationRow {
    operation_id: String,
    session_id: String,
    request_sha256: String,
    job_id: String,
}

#[derive(Debug, Deserialize)]
struct CandidateRow {
    organization_id: String,
    upload_id: String,
    video_id: String,
    source_object_key: String,
    source_version: i64,
    expected_bytes: i64,
    upload_content_type: String,
    session_state: String,
    request_parts_sha256: String,
    provider_version: String,
    provider_etag: String,
    bytes: i64,
    checksum_sha256: String,
    content_type: String,
    duration_ms: i64,
    probe_matches: i64,
    integration_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PendingCandidate {
    pub(crate) session_id: String,
    pub(crate) organization_id: String,
}

pub(crate) async fn retain_request(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    http_idempotency_key: &str,
    request: &InstantFinalizeRequestV1,
    now_ms: i64,
) -> Result<InstantFinalizeReceiptV1, InstantFinalizeFailure> {
    request
        .validate()
        .map_err(|_| InstantFinalizeFailure::Conflict)?;

    if let Some(reservation) =
        load_reservation(database, &request.tenant_id, http_idempotency_key).await?
    {
        if reservation.operation_id != request.operation_id
            || reservation.session_id != request.session_id
            || reservation.request_sha256 != request.request_sha256
            || reservation.job_id != request.job_id
        {
            return Err(InstantFinalizeFailure::Conflict);
        }
        return load_request(database, &request.session_id)
            .await?
            .filter(|row| row.exact(request))
            .ok_or(InstantFinalizeFailure::Conflict)?
            .receipt();
    }

    if database
        .prepare(
            "SELECT 1 AS present FROM instant_finalize_operations_v1 \
             WHERE operation_id=?1 LIMIT 1",
        )
        .bind(&[JsValue::from_str(&request.operation_id)])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .first::<PresenceRow>(None)
        .await
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .is_some()
    {
        return Err(InstantFinalizeFailure::Conflict);
    }
    if let Some(existing) = load_request(database, &request.session_id).await?
        && (!existing.exact(request) || existing.state == "dead_letter")
    {
        return Err(InstantFinalizeFailure::Conflict);
    }

    let statements = vec![
        database
            .prepare(
                "INSERT INTO instant_finalize_requests_v1(\
                 session_id,organization_id,upload_id,video_id,ordered_parts_sha256,object_version,\
                 job_id,job_generation,request_sha256,state,publication_id,playable_object_key,\
                 distribution_eligible,reconcile_attempt_count,next_attempt_at_ms,last_failure_class,\
                 created_at_ms,updated_at_ms,published_at_ms,dead_lettered_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',NULL,NULL,0,0,?10,NULL,?10,?10,NULL,NULL) \
                 ON CONFLICT(session_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&request.session_id),
                JsValue::from_str(&request.tenant_id),
                JsValue::from_str(&request.upload_id),
                JsValue::from_str(&request.video_id),
                JsValue::from_str(&request.ordered_parts_sha256),
                JsValue::from_str(&request.object_version),
                JsValue::from_str(&request.job_id),
                JsValue::from_f64(request.job_generation as f64),
                JsValue::from_str(&request.request_sha256),
                JsValue::from_f64(now_ms as f64),
            ])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
        database
            .prepare(
                "INSERT INTO instant_finalize_jobs_v1(\
                 job_id,session_id,generation,request_sha256,state,created_at_ms,updated_at_ms) \
                 VALUES(?1,?2,?3,?4,'retained',?5,?5) ON CONFLICT(job_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&request.job_id),
                JsValue::from_str(&request.session_id),
                JsValue::from_f64(request.job_generation as f64),
                JsValue::from_str(&request.request_sha256),
                JsValue::from_f64(now_ms as f64),
            ])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
        database
            .prepare(
                "INSERT INTO instant_finalize_operations_v1(\
                 operation_id,session_id,request_sha256,result_state,publication_id,committed_at_ms) \
                 SELECT ?1,r.session_id,r.request_sha256,r.state,r.publication_id,?5 \
                 FROM instant_finalize_requests_v1 r \
                 WHERE r.session_id=?2 AND r.organization_id=?3 AND r.request_sha256=?4 \
                 ON CONFLICT(operation_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&request.operation_id),
                JsValue::from_str(&request.session_id),
                JsValue::from_str(&request.tenant_id),
                JsValue::from_str(&request.request_sha256),
                JsValue::from_f64(now_ms as f64),
            ])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
        database
            .prepare(
                "INSERT INTO instant_finalize_http_idempotency_v1(\
                 organization_id,idempotency_key,operation_id,session_id,request_sha256,job_id,created_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7) \
                 ON CONFLICT(organization_id,idempotency_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&request.tenant_id),
                JsValue::from_str(http_idempotency_key),
                JsValue::from_str(&request.operation_id),
                JsValue::from_str(&request.session_id),
                JsValue::from_str(&request.request_sha256),
                JsValue::from_str(&request.job_id),
                JsValue::from_f64(now_ms as f64),
            ])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
        database
            .prepare(
                "INSERT INTO instant_finalize_reservation_assertions_v1(\
                 operation_id,organization_id,idempotency_key,asserted_at_ms) \
                 VALUES(?1,?2,?3,?4) ON CONFLICT(operation_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&request.operation_id),
                JsValue::from_str(&request.tenant_id),
                JsValue::from_str(http_idempotency_key),
                JsValue::from_f64(now_ms as f64),
            ])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
    ];
    execute_fenced_batch(
        database,
        authority_fence,
        &format!("instant-retain:{}", request.operation_id),
        now_ms,
        statements,
    )
    .await?;

    let reservation = load_reservation(database, &request.tenant_id, http_idempotency_key)
        .await?
        .filter(|row| {
            row.operation_id == request.operation_id
                && row.session_id == request.session_id
                && row.request_sha256 == request.request_sha256
                && row.job_id == request.job_id
        })
        .ok_or(InstantFinalizeFailure::Conflict)?;
    let _ = reservation;
    load_request(database, &request.session_id)
        .await?
        .filter(|row| row.exact(request))
        .ok_or(InstantFinalizeFailure::Conflict)?
        .receipt()
}

pub(crate) async fn reconcile_session(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    session_id: &str,
    now_ms: i64,
) -> Result<InstantFinalizeReceiptV1, InstantFinalizeFailure> {
    let request = load_request(database, session_id)
        .await?
        .ok_or(InstantFinalizeFailure::Conflict)?;
    if request.state == "published" {
        return request.receipt();
    }
    if request.state != "pending" {
        return Err(InstantFinalizeFailure::Conflict);
    }
    let candidate = load_candidate(database, session_id)
        .await?
        .ok_or(InstantFinalizeFailure::Pending)?;
    if candidate.organization_id != request.organization_id
        || candidate.upload_id != request.upload_id
        || candidate.video_id != request.video_id
        || candidate.session_state != "complete"
        || candidate.request_parts_sha256 != request.ordered_parts_sha256
        || candidate.expected_bytes != candidate.bytes
        || candidate.upload_content_type != candidate.content_type
        || candidate.probe_matches != 1
        || object_version(&candidate.provider_version) != request.object_version
    {
        return Err(InstantFinalizeFailure::Conflict);
    }

    let publication_id = Uuid::now_v7().to_string();
    let storage_object_id = Uuid::now_v7().to_string();
    let upload_fingerprints = [
        event_fingerprint(&request.upload_id, "uploading", &candidate.checksum_sha256),
        event_fingerprint(&request.upload_id, "finalizing", &candidate.checksum_sha256),
        event_fingerprint(&request.upload_id, "complete", &candidate.checksum_sha256),
    ];
    let statements = vec![
        database.prepare(
            "UPDATE video_uploads SET state='uploading',updated_at_ms=?3,revision=revision+1,\
             event_sequence=event_sequence+1,event_fingerprint=?4 \
             WHERE id=?1 AND organization_id=?2 AND state='initiated'",
        ).bind(&[
            JsValue::from_str(&request.upload_id), JsValue::from_str(&request.organization_id),
            JsValue::from_f64(now_ms as f64), JsValue::from_str(&upload_fingerprints[0]),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "UPDATE video_uploads SET state='finalizing',updated_at_ms=?3,revision=revision+1,\
             event_sequence=event_sequence+1,event_fingerprint=?4 \
             WHERE id=?1 AND organization_id=?2 AND state='uploading'",
        ).bind(&[
            JsValue::from_str(&request.upload_id), JsValue::from_str(&request.organization_id),
            JsValue::from_f64(now_ms as f64), JsValue::from_str(&upload_fingerprints[1]),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "UPDATE video_uploads SET state='complete',received_bytes=expected_bytes,checksum_sha256=?3,\
             updated_at_ms=?4,revision=revision+1,event_sequence=event_sequence+1,event_fingerprint=?5 \
             WHERE id=?1 AND organization_id=?2 AND state='finalizing'",
        ).bind(&[
            JsValue::from_str(&request.upload_id), JsValue::from_str(&request.organization_id),
            JsValue::from_str(&candidate.checksum_sha256), JsValue::from_f64(now_ms as f64),
            JsValue::from_str(&upload_fingerprints[2]),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "INSERT INTO object_manifests(object_key,video_id,role,bytes,checksum_sha256,content_type,\
             created_at_ms,organization_id,object_version,provider_etag,state,updated_at_ms) \
             VALUES(?1,?2,'source',?3,?4,?5,?6,?7,?8,?9,'available',?6) \
             ON CONFLICT(object_key) DO NOTHING",
        ).bind(&[
            JsValue::from_str(&candidate.source_object_key), JsValue::from_str(&request.video_id),
            JsValue::from_f64(candidate.bytes as f64), JsValue::from_str(&candidate.checksum_sha256),
            JsValue::from_str(&candidate.content_type), JsValue::from_f64(now_ms as f64),
            JsValue::from_str(&request.organization_id), JsValue::from_f64(candidate.source_version as f64),
            JsValue::from_str(&candidate.provider_etag),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "INSERT INTO storage_objects(id,organization_id,integration_id,video_id,object_key,role,\
             object_version,state,bytes,content_type,checksum_sha256,provider_etag,created_at_ms) \
             VALUES(?1,?2,?3,?4,?5,'source',?6,'available',?7,?8,?9,?10,?11) \
             ON CONFLICT(integration_id,object_key) DO NOTHING",
        ).bind(&[
            JsValue::from_str(&storage_object_id), JsValue::from_str(&request.organization_id),
            JsValue::from_str(&candidate.integration_id), JsValue::from_str(&request.video_id),
            JsValue::from_str(&candidate.source_object_key), JsValue::from_f64(candidate.source_version as f64),
            JsValue::from_f64(candidate.bytes as f64), JsValue::from_str(&candidate.content_type),
            JsValue::from_str(&candidate.checksum_sha256), JsValue::from_str(&candidate.provider_etag),
            JsValue::from_f64(now_ms as f64),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "INSERT INTO storage_governed_objects_v1(organization_id,object_key,role,visibility,state,\
             malware_disposition,immutable_revision,cache_generation,checksum_sha256,bytes,content_type,\
             retention_until_ms,created_at_ms,updated_at_ms) \
             VALUES(?1,?2,'source','private','active','clean',?3,1,?4,?5,?6,NULL,?7,?7) \
             ON CONFLICT(organization_id,object_key) DO NOTHING",
        ).bind(&[
            JsValue::from_str(&request.organization_id), JsValue::from_str(&candidate.source_object_key),
            JsValue::from_f64(candidate.source_version as f64), JsValue::from_str(&candidate.checksum_sha256),
            JsValue::from_f64(candidate.bytes as f64), JsValue::from_str(&candidate.content_type),
            JsValue::from_f64(now_ms as f64),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "UPDATE videos SET source_object_key=?3,playback_object_key=?3,duration_ms=?4,state='ready',\
             updated_at_ms=?5,revision=revision+1 WHERE id=?1 AND organization_id=?2 \
             AND deleted_at_ms IS NULL",
        ).bind(&[
            JsValue::from_str(&request.video_id), JsValue::from_str(&request.organization_id),
            JsValue::from_str(&candidate.source_object_key), JsValue::from_f64(candidate.duration_ms as f64),
            JsValue::from_f64(now_ms as f64),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "UPDATE instant_finalize_requests_v1 SET state='published',publication_id=?2,\
             playable_object_key=?3,distribution_eligible=1,updated_at_ms=?4,published_at_ms=?4,\
             last_failure_class=NULL WHERE session_id=?1 AND state='pending'",
        ).bind(&[
            JsValue::from_str(session_id), JsValue::from_str(&publication_id),
            JsValue::from_str(&candidate.source_object_key), JsValue::from_f64(now_ms as f64),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "UPDATE instant_finalize_jobs_v1 SET state='published',updated_at_ms=?2 \
             WHERE session_id=?1 AND state='retained'",
        ).bind(&[JsValue::from_str(session_id), JsValue::from_f64(now_ms as f64)])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "UPDATE instant_finalize_operations_v1 SET result_state='published',\
             publication_id=(SELECT publication_id FROM instant_finalize_requests_v1 WHERE session_id=?1) \
             WHERE session_id=?1 AND result_state='pending'",
        ).bind(&[JsValue::from_str(session_id)])
            .map_err(|_| InstantFinalizeFailure::Persistence)?,
        database.prepare(
            "INSERT INTO instant_finalize_publication_assertions_v1(\
             session_id,publication_id,asserted_at_ms) VALUES(?1,?2,?3) \
             ON CONFLICT(session_id) DO NOTHING",
        ).bind(&[
            JsValue::from_str(session_id), JsValue::from_str(&publication_id),
            JsValue::from_f64(now_ms as f64),
        ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
    ];
    execute_fenced_batch(
        database,
        authority_fence,
        &format!("instant-publish:{session_id}:{publication_id}"),
        now_ms,
        statements,
    )
    .await?;

    let published = load_request(database, session_id)
        .await?
        .filter(|row| {
            row.state == "published" && row.publication_id.as_deref() == Some(&publication_id)
        })
        .ok_or(InstantFinalizeFailure::Persistence)?;
    ensure_publication_postcondition(database, &published).await?;
    published.receipt()
}

pub(crate) async fn scan_candidates(
    database: &D1Database,
    now_ms: i64,
    limit: u16,
) -> Result<Vec<PendingCandidate>, InstantFinalizeFailure> {
    if !(1..=64).contains(&limit) {
        return Err(InstantFinalizeFailure::Persistence);
    }
    database
        .prepare(
            "SELECT r.session_id,r.organization_id FROM instant_finalize_requests_v1 r \
             CROSS JOIN instant_finalize_scheduler_v1 scheduler \
             WHERE scheduler.singleton=1 AND r.state='pending' AND r.next_attempt_at_ms<=?1 \
             ORDER BY CASE WHEN scheduler.cursor_session_id IS NULL \
               OR r.session_id>scheduler.cursor_session_id THEN 0 ELSE 1 END,r.session_id LIMIT ?2",
        )
        .bind(&[
            JsValue::from_f64(now_ms as f64),
            JsValue::from_f64(f64::from(limit)),
        ])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .all()
        .await
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .results::<PendingCandidate>()
        .map_err(|_| InstantFinalizeFailure::Persistence)
}

pub(crate) async fn advance_cursor(
    database: &D1Database,
    session_id: &str,
    now_ms: i64,
) -> Result<(), InstantFinalizeFailure> {
    let result = database
        .prepare(
            "UPDATE instant_finalize_scheduler_v1 SET cursor_session_id=?1,updated_at_ms=?2 \
             WHERE singleton=1",
        )
        .bind(&[
            JsValue::from_str(session_id),
            JsValue::from_f64(now_ms as f64),
        ])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .run()
        .await
        .map_err(|_| InstantFinalizeFailure::Persistence)?;
    if result.success() {
        Ok(())
    } else {
        Err(InstantFinalizeFailure::Persistence)
    }
}

pub(crate) async fn record_reconcile_failure(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    session_id: &str,
    failure: InstantFinalizeFailure,
    now_ms: i64,
) -> Result<(), InstantFinalizeFailure> {
    let row = load_request(database, session_id)
        .await?
        .filter(|row| row.state == "pending")
        .ok_or(InstantFinalizeFailure::Conflict)?;
    let attempt = row
        .reconcile_attempt_count
        .checked_add(1)
        .ok_or(InstantFinalizeFailure::Persistence)?;
    let terminal = failure == InstantFinalizeFailure::Conflict || attempt >= MAX_RECONCILE_ATTEMPTS;
    let failure_class = failure.retained_class();

    if terminal {
        let statements = vec![
            database.prepare(
                "UPDATE instant_finalize_requests_v1 SET state='dead_letter',\
                 reconcile_attempt_count=?2,next_attempt_at_ms=?3,last_failure_class=?4,\
                 updated_at_ms=?3,dead_lettered_at_ms=?3 \
                 WHERE session_id=?1 AND state='pending' AND reconcile_attempt_count=?5",
            ).bind(&[
                JsValue::from_str(session_id), JsValue::from_f64(attempt as f64),
                JsValue::from_f64(now_ms as f64), JsValue::from_str(failure_class),
                JsValue::from_f64(row.reconcile_attempt_count as f64),
            ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
            database.prepare(
                "UPDATE instant_finalize_jobs_v1 SET state='cancelled',updated_at_ms=?2 \
                 WHERE session_id=?1 AND state='retained' AND EXISTS(\
                   SELECT 1 FROM instant_finalize_requests_v1 r \
                   WHERE r.session_id=?1 AND r.state='dead_letter')",
            ).bind(&[JsValue::from_str(session_id), JsValue::from_f64(now_ms as f64)])
                .map_err(|_| InstantFinalizeFailure::Persistence)?,
            database.prepare(
                "UPDATE instant_finalize_operations_v1 SET result_state='dead_letter' \
                 WHERE session_id=?1 AND result_state='pending' AND EXISTS(\
                   SELECT 1 FROM instant_finalize_requests_v1 r \
                   WHERE r.session_id=?1 AND r.state='dead_letter')",
            ).bind(&[JsValue::from_str(session_id)])
                .map_err(|_| InstantFinalizeFailure::Persistence)?,
            database.prepare(
                "INSERT INTO instant_finalize_dead_letters_v1(\
                 session_id,organization_id,request_sha256,attempt_count,failure_class,created_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id) DO NOTHING",
            ).bind(&[
                JsValue::from_str(session_id), JsValue::from_str(&row.organization_id),
                JsValue::from_str(&row.request_sha256), JsValue::from_f64(attempt as f64),
                JsValue::from_str(failure_class), JsValue::from_f64(now_ms as f64),
            ]).map_err(|_| InstantFinalizeFailure::Persistence)?,
        ];
        execute_fenced_batch(
            database,
            authority_fence,
            &format!("instant-dead-letter:{session_id}:{attempt}"),
            now_ms,
            statements,
        )
        .await?;
        return load_request(database, session_id)
            .await?
            .filter(|stored| {
                stored.state == "dead_letter"
                    && stored.reconcile_attempt_count == attempt
                    && stored.last_failure_class.as_deref() == Some(failure_class)
            })
            .map(|_| ())
            .ok_or(InstantFinalizeFailure::Persistence);
    }

    let next_attempt_at_ms = next_attempt_at(now_ms, attempt);
    let statement = database
        .prepare(
            "UPDATE instant_finalize_requests_v1 SET reconcile_attempt_count=?2,\
         next_attempt_at_ms=?3,last_failure_class=?4,updated_at_ms=?5 \
         WHERE session_id=?1 AND state='pending' AND reconcile_attempt_count=?6",
        )
        .bind(&[
            JsValue::from_str(session_id),
            JsValue::from_f64(attempt as f64),
            JsValue::from_f64(next_attempt_at_ms as f64),
            JsValue::from_str(failure_class),
            JsValue::from_f64(now_ms as f64),
            JsValue::from_f64(row.reconcile_attempt_count as f64),
        ])
        .map_err(|_| InstantFinalizeFailure::Persistence)?;
    execute_fenced_batch(
        database,
        authority_fence,
        &format!("instant-retry:{session_id}:{attempt}"),
        now_ms,
        vec![statement],
    )
    .await
}

async fn load_reservation(
    database: &D1Database,
    organization_id: &str,
    idempotency_key: &str,
) -> Result<Option<ReservationRow>, InstantFinalizeFailure> {
    database
        .prepare(
            "SELECT h.operation_id,h.session_id,h.request_sha256,h.job_id \
             FROM instant_finalize_http_idempotency_v1 h \
             JOIN instant_finalize_reservation_assertions_v1 a \
               ON a.operation_id=h.operation_id AND a.organization_id=h.organization_id \
               AND a.idempotency_key=h.idempotency_key \
             WHERE h.organization_id=?1 AND h.idempotency_key=?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(organization_id),
            JsValue::from_str(idempotency_key),
        ])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .first::<ReservationRow>(None)
        .await
        .map_err(|_| InstantFinalizeFailure::Persistence)
}

async fn load_request(
    database: &D1Database,
    session_id: &str,
) -> Result<Option<RequestRow>, InstantFinalizeFailure> {
    database
        .prepare(
            "SELECT session_id,organization_id,upload_id,video_id,ordered_parts_sha256,\
             object_version,job_id,job_generation,request_sha256,state,publication_id,\
             playable_object_key,distribution_eligible,reconcile_attempt_count,last_failure_class \
             FROM instant_finalize_requests_v1 WHERE session_id=?1 LIMIT 1",
        )
        .bind(&[JsValue::from_str(session_id)])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .first::<RequestRow>(None)
        .await
        .map_err(|_| InstantFinalizeFailure::Persistence)
}

async fn load_candidate(
    database: &D1Database,
    session_id: &str,
) -> Result<Option<CandidateRow>, InstantFinalizeFailure> {
    database.prepare(
        "SELECT r.organization_id,r.upload_id,r.video_id,u.source_object_key,u.source_version,\
         u.expected_bytes,u.content_type AS upload_content_type,s.state AS session_state,\
         c.request_parts_sha256,c.provider_version,c.provider_etag,c.bytes,c.checksum_sha256,\
         c.content_type,c.duration_ms,\
         CASE WHEN EXISTS(SELECT 1 FROM media_source_probes_v1 p \
           WHERE p.organization_id=r.organization_id AND p.video_id=r.video_id \
             AND p.source_version=u.source_version AND p.source_object_key=u.source_object_key \
             AND p.source_checksum_sha256=c.checksum_sha256 AND p.source_bytes=c.bytes \
             AND p.source_content_type=c.content_type AND p.container=c.container \
             AND p.video_codec=c.video_codec AND p.audio_codec=c.audio_codec \
             AND p.duration_ms=c.duration_ms AND p.width=c.width AND p.height=c.height \
             AND CAST((p.frame_rate_numerator * 1000) / p.frame_rate_denominator AS INTEGER) \
                 BETWEEN c.frame_rate_millihertz - 1 AND c.frame_rate_millihertz + 1 \
             AND p.trust='verified_native_probe' AND p.state='verified') THEN 1 ELSE 0 END AS probe_matches,\
         i.id AS integration_id \
         FROM instant_finalize_requests_v1 r \
         JOIN video_uploads u ON u.id=r.upload_id AND u.organization_id=r.organization_id \
         JOIN r2_multipart_intents_v1 intent ON intent.upload_id=r.upload_id \
         JOIN r2_multipart_sessions_v1 s ON s.upload_id=r.upload_id AND s.object_key=u.source_object_key \
         JOIN r2_multipart_completions_v1 c ON c.upload_id=s.upload_id \
         JOIN r2_multipart_verified_objects_v1 verified ON verified.upload_id=s.upload_id \
           AND verified.provider_version=c.provider_version AND verified.provider_etag=c.provider_etag \
           AND verified.bytes=c.bytes AND verified.checksum_sha256=c.checksum_sha256 \
           AND verified.content_type=c.content_type \
         JOIN storage_integrations i ON i.id=intent.integration_id \
           AND i.organization_id=r.organization_id AND i.provider='r2' AND i.state='active' \
           AND json_extract(i.capabilities_json,'$.multipart')=1 \
         WHERE r.session_id=?1 AND r.state='pending' ORDER BY i.created_at_ms LIMIT 1",
    ).bind(&[JsValue::from_str(session_id)])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .first::<CandidateRow>(None).await
        .map_err(|_| InstantFinalizeFailure::Persistence)
}

async fn ensure_publication_postcondition(
    database: &D1Database,
    request: &RequestRow,
) -> Result<(), InstantFinalizeFailure> {
    let present = database
        .prepare(
            "SELECT 1 AS present FROM instant_finalize_publication_assertions_v1 a \
         WHERE a.session_id=?1 AND a.publication_id=?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&request.session_id),
            request
                .publication_id
                .as_deref()
                .map_or(JsValue::NULL, JsValue::from_str),
        ])
        .map_err(|_| InstantFinalizeFailure::Persistence)?
        .first::<PresenceRow>(None)
        .await
        .map_err(|_| InstantFinalizeFailure::Persistence)?;
    if present.is_some_and(|row| row.present == 1) {
        Ok(())
    } else {
        Err(InstantFinalizeFailure::Persistence)
    }
}

#[derive(Debug, Deserialize)]
struct PresenceRow {
    present: i64,
}

async fn execute_fenced_batch(
    database: &D1Database,
    authority_fence: &crate::MutationAuthorityFence,
    operation_id: &str,
    now_ms: i64,
    statements: Vec<D1PreparedStatement>,
) -> Result<(), InstantFinalizeFailure> {
    let expected = statements.len();
    let results =
        crate::execute_mutation_batch(database, authority_fence, operation_id, now_ms, statements)
            .await
            .map_err(|_| InstantFinalizeFailure::Persistence)?;
    if results.len() != expected || results.iter().any(|result| !result.success()) {
        return Err(InstantFinalizeFailure::Persistence);
    }
    Ok(())
}

fn next_attempt_at(now_ms: i64, attempt: i64) -> i64 {
    let shift = u32::try_from(attempt.saturating_sub(1).min(18)).unwrap_or(18);
    let delay = RETRY_BASE_MS
        .saturating_mul(1_i64.checked_shl(shift).unwrap_or(i64::MAX))
        .min(RETRY_MAX_MS);
    now_ms.saturating_add(delay)
}

fn object_version(provider_version: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame.instant.r2-object-version.v1\0");
    digest.update((provider_version.len() as u32).to_be_bytes());
    digest.update(provider_version.as_bytes());
    hex(&digest.finalize())
}

fn event_fingerprint(upload_id: &str, state: &str, checksum: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame.instant.upload-event.v1\0");
    for value in [upload_id, state, checksum] {
        digest.update((value.len() as u32).to_be_bytes());
        digest.update(value.as_bytes());
    }
    hex(&digest.finalize())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_version_and_event_namespaces_are_stable_and_separate() {
        assert_eq!(object_version("v1").len(), 64);
        assert_eq!(
            event_fingerprint("u", "complete", "c"),
            event_fingerprint("u", "complete", "c")
        );
        assert_ne!(
            event_fingerprint("u", "uploading", "c"),
            event_fingerprint("u", "complete", "c")
        );
        assert_ne!(object_version("v1"), object_version("v2"));
    }

    #[test]
    fn retry_backoff_is_bounded_and_monotonic() {
        let mut previous = 0;
        for attempt in 1..=MAX_RECONCILE_ATTEMPTS {
            let next = next_attempt_at(0, attempt);
            assert!(next >= previous);
            assert!(next <= RETRY_MAX_MS);
            previous = next;
        }
        assert_eq!(next_attempt_at(i64::MAX - 1, 8), i64::MAX);
    }
}
