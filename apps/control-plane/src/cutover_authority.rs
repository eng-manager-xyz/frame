//! Tenant/domain-scoped cutover authority for the D1 control plane.
//!
//! This adapter deliberately keeps the released global singleton visible as a
//! compatibility signal while authorizing writes only from the scoped
//! `(tenant, domain, writer, epoch)` row. Callers place the assertion and their
//! mutation in one D1 batch through [`execute_fenced_batch`], so a transition
//! cannot race between an authority read and the write it authorized.

use std::fmt;

use frame_domain::{
    AuthorityFence, CutoverEvidence, CutoverPhase, CutoverScope, CutoverState, DataAuthority,
    MAX_WIRE_INTEGER, TimestampMillis,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const AUTHORITY_SNAPSHOT_SQL: &str = include_str!("../queries/cutover/authority_snapshot.sql");
const WRITER_ASSERT_SQL: &str = include_str!("../queries/cutover/writer_assert.sql");
const ASSERTION_CLEANUP_SQL: &str = include_str!("../queries/cutover/assertion_cleanup.sql");
const WRITER_STATE_SQL: &str = include_str!("../queries/cutover/writer_state.sql");
const SIGNAL_INSERT_SQL: &str = include_str!("../queries/cutover/signal_insert.sql");
const SHADOW_OBSERVATION_INSERT_SQL: &str =
    include_str!("../queries/cutover/shadow_observation_insert.sql");
const TRANSITION_ASSERT_SQL: &str = include_str!("../queries/cutover/transition_assert.sql");
const CONTROL_ASSERT_SQL: &str = include_str!("../queries/cutover/control_assert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/cutover/audit_insert.sql");
const SCOPE_TRANSITION_SQL: &str = include_str!("../queries/cutover/scope_transition.sql");
const SCOPE_CONTROL_SQL: &str = include_str!("../queries/cutover/scope_control.sql");
const STATE_POSTCONDITION_SQL: &str = include_str!("../queries/cutover/state_postcondition.sql");

const MAX_FENCED_MUTATIONS: usize = 96;
const MAX_OPERATION_ID_BYTES: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverAuthorityFailure {
    InvalidRequest,
    NotFound,
    StaleAuthority,
    MutationRejected,
    Unavailable,
    Corrupt,
}

impl CutoverAuthorityFailure {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "cutover_authority_invalid_request",
            Self::NotFound => "cutover_authority_not_found",
            Self::StaleAuthority => "cutover_authority_stale",
            Self::MutationRejected => "cutover_authority_mutation_rejected",
            Self::Unavailable => "cutover_authority_unavailable",
            Self::Corrupt => "cutover_authority_corrupt",
        }
    }

    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::StaleAuthority | Self::MutationRejected | Self::Unavailable
        )
    }
}

impl fmt::Display for CutoverAuthorityFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for CutoverAuthorityFailure {}

pub type CutoverAuthorityResult<T> = Result<T, CutoverAuthorityFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverSignalKind {
    AuthorityContention,
    ReplayWriteFailure,
    ReplayLostAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowClassification {
    Match,
    OrderingOnly,
    SemanticMismatch,
    Missing,
    Error,
}

impl ShadowClassification {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::OrderingOnly => "ordering_only",
            Self::SemanticMismatch => "semantic_mismatch",
            Self::Missing => "missing",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutoverShadowObservation {
    pub scope: CutoverScope,
    pub phase_epoch: u64,
    pub observation_digest: String,
    pub query_class: String,
    pub normalization_digest: String,
    pub legacy_result_digest: String,
    pub d1_result_digest: String,
    pub classification: ShadowClassification,
    pub observed_at: TimestampMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayControlAction {
    Pause,
    Resume,
}

impl ReplayControlAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }

    #[must_use]
    pub const fn paused(self) -> bool {
        matches!(self, Self::Pause)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedCutoverTransition {
    pub scope: CutoverScope,
    pub target: CutoverPhase,
    pub expected_epoch: u64,
    pub operator_digest: String,
    pub evidence: CutoverEvidence,
    pub reconciliation_digest: Option<String>,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedReplayControl {
    pub scope: CutoverScope,
    pub action: ReplayControlAction,
    pub expected_epoch: u64,
    pub operator_digest: String,
    pub occurred_at: TimestampMillis,
}

impl CutoverSignalKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityContention => "authority_contention",
            Self::ReplayWriteFailure => "replay_write_failure",
            Self::ReplayLostAck => "replay_lost_ack",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CutoverSignalWindow {
    pub authority_contention: u64,
    pub replay_write_failures: u64,
    pub replay_lost_ack: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CutoverReplayHealth {
    pub pending_events: u64,
    pub dead_letter_events: u64,
    pub pending_lag_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CutoverShadowHealth {
    pub window_started_at_ms: u64,
    pub required_query_classes: u64,
    pub covered_query_classes: u64,
    pub observations: u64,
    pub mismatches: u64,
}

impl CutoverShadowHealth {
    #[must_use]
    pub const fn coverage_complete(self) -> bool {
        self.required_query_classes > 0 && self.covered_query_classes == self.required_query_classes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CutoverSloPolicy {
    pub shadow_window_ms: u64,
    pub minimum_shadow_observations: u64,
    pub max_pending_lag_ms: u64,
    pub max_shadow_mismatches: u64,
    pub max_dead_letter_events: u64,
    pub max_contention_events: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CompatibilityAuthority {
    pub phase: CutoverPhase,
    pub writer: DataAuthority,
    pub epoch: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CutoverAuthoritySnapshot {
    pub scope: CutoverScope,
    pub phase: CutoverPhase,
    pub writer: DataAuthority,
    pub mirror_enabled: bool,
    pub replay_paused: bool,
    pub epoch: u64,
    pub phase_epoch: u64,
    pub audit_head: String,
    pub rollback_ready: bool,
    pub phase_started_at_ms: u64,
    pub updated_at_ms: u64,
    pub compatibility: CompatibilityAuthority,
    pub slo: CutoverSloPolicy,
    pub shadow: CutoverShadowHealth,
    pub replay: CutoverReplayHealth,
    pub signals: CutoverSignalWindow,
    pub maintenance_window_active: bool,
}

impl CutoverAuthoritySnapshot {
    #[must_use]
    pub const fn compatibility_conflict(&self) -> bool {
        matches!(self.compatibility.writer, DataAuthority::D1)
            && matches!(self.writer, DataAuthority::Legacy)
    }

    #[must_use]
    pub const fn promotion_health_is_clean(&self) -> bool {
        self.forward_health_is_clean(true)
    }

    #[must_use]
    pub const fn forward_health_is_clean(&self, require_drained_replay: bool) -> bool {
        self.shadow.coverage_complete()
            && self.shadow.mismatches == 0
            && self.replay.pending_lag_ms <= self.slo.max_pending_lag_ms
            && self.replay.dead_letter_events <= self.slo.max_dead_letter_events
            && self.signals.authority_contention <= self.slo.max_contention_events
            && self.signals.replay_write_failures == 0
            && self.signals.replay_lost_ack == 0
            && !self.compatibility_conflict()
            && (!require_drained_replay
                || (self.replay.pending_events == 0 && self.replay.dead_letter_events == 0))
    }

    pub fn authorize_writer(
        &self,
        writer: DataAuthority,
        expected_epoch: u64,
    ) -> CutoverAuthorityResult<AuthorityFence> {
        if self.writer != writer
            || self.epoch != expected_epoch
            || (matches!(writer, DataAuthority::Legacy) && self.compatibility_conflict())
        {
            return Err(CutoverAuthorityFailure::StaleAuthority);
        }
        Ok(AuthorityFence {
            scope: self.scope.clone(),
            writer,
            epoch: expected_epoch,
        })
    }
}

#[derive(Debug, Deserialize)]
struct AuthoritySnapshotRow {
    tenant_id: String,
    domain: String,
    phase: String,
    writer: String,
    mirror_enabled: i64,
    replay_paused: i64,
    epoch: i64,
    phase_epoch: i64,
    audit_head: String,
    rollback_ready: i64,
    phase_started_at_ms: i64,
    updated_at_ms: i64,
    singleton_phase: String,
    singleton_authority: String,
    singleton_epoch: i64,
    singleton_updated_at_ms: i64,
    shadow_window_ms: i64,
    minimum_shadow_observations: i64,
    max_pending_lag_ms: i64,
    max_shadow_mismatches: i64,
    max_dead_letter_events: i64,
    max_contention_events: i64,
    approved_by_digest: String,
    slo_updated_at_ms: i64,
    observation_window_started_at_ms: i64,
    required_query_classes: i64,
    covered_query_classes: i64,
    shadow_observations: i64,
    shadow_mismatches: i64,
    pending_events: i64,
    dead_letter_events: i64,
    pending_lag_ms: i64,
    authority_contention_events: i64,
    replay_write_failures: i64,
    replay_lost_ack_events: i64,
    signal_rollup_consistent: i64,
    audit_head_consistent: i64,
    maintenance_window_active: i64,
}

#[derive(Debug, Deserialize)]
struct WriterStateRow {
    tenant_id: String,
    domain: String,
    phase: String,
    writer: String,
    epoch: i64,
    phase_epoch: i64,
    audit_head: String,
    updated_at_ms: i64,
}

impl AuthoritySnapshotRow {
    fn decode(
        self,
        expected_scope: &CutoverScope,
        observed_at: TimestampMillis,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        let now = u64::try_from(observed_at.get())
            .map_err(|_| CutoverAuthorityFailure::InvalidRequest)?;
        if self.tenant_id != expected_scope.tenant_id.to_string()
            || self.domain != expected_scope.domain.as_str()
            || !lower_sha256(&self.audit_head)
            || !lower_sha256(&self.approved_by_digest)
        {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        let phase = parse_phase(&self.phase)?;
        let writer = parse_writer(&self.writer)?;
        let mirror_enabled = boolean(self.mirror_enabled)?;
        let replay_paused = boolean(self.replay_paused)?;
        let rollback_ready = boolean(self.rollback_ready)?;
        let epoch = wire(self.epoch)?;
        let phase_epoch = wire(self.phase_epoch)?;
        let phase_started_at_ms = wire(self.phase_started_at_ms)?;
        let updated_at_ms = wire(self.updated_at_ms)?;
        let singleton_phase = parse_phase(&self.singleton_phase)?;
        let singleton_writer =
            parse_compatibility_writer(singleton_phase, &self.singleton_authority)?;
        let singleton_epoch = wire(self.singleton_epoch)?;
        let singleton_updated_at_ms = wire(self.singleton_updated_at_ms)?;
        let slo_updated_at_ms = wire(self.slo_updated_at_ms)?;
        let observation_window_started_at_ms = wire(self.observation_window_started_at_ms)?;
        let required_query_classes = wire(self.required_query_classes)?;
        let covered_query_classes = wire(self.covered_query_classes)?;
        let shadow_observations = wire(self.shadow_observations)?;
        let shadow_mismatches = wire(self.shadow_mismatches)?;
        let pending_events = wire(self.pending_events)?;
        let dead_letter_events = wire(self.dead_letter_events)?;
        let pending_lag_ms = wire(self.pending_lag_ms)?;
        let authority_contention = wire(self.authority_contention_events)?;
        let replay_write_failures = wire(self.replay_write_failures)?;
        let replay_lost_ack = wire(self.replay_lost_ack_events)?;
        let shadow_window_ms = positive_wire(self.shadow_window_ms)?;
        let minimum_shadow_observations = positive_wire(self.minimum_shadow_observations)?;
        let max_pending_lag_ms = positive_wire(self.max_pending_lag_ms)?;
        let max_shadow_mismatches = wire(self.max_shadow_mismatches)?;
        let max_dead_letter_events = wire(self.max_dead_letter_events)?;
        let max_contention_events = wire(self.max_contention_events)?;
        let signal_rollup_consistent = boolean(self.signal_rollup_consistent)?;
        let audit_head_consistent = boolean(self.audit_head_consistent)?;
        if writer != phase_writer(phase)
            || mirror_enabled != mirror_phase(phase)
            || rollback_ready != matches!(phase, CutoverPhase::D1Authoritative)
            || phase_epoch > epoch
            || (replay_paused
                && matches!(
                    phase,
                    CutoverPhase::LegacyAuthoritative | CutoverPhase::Finalized
                ))
            || singleton_writer != phase_writer(singleton_phase)
            || phase_started_at_ms > updated_at_ms
            || updated_at_ms > now
            || singleton_updated_at_ms > now
            || slo_updated_at_ms > now
            || observation_window_started_at_ms < phase_started_at_ms
            || observation_window_started_at_ms > now
            || covered_query_classes > required_query_classes
            || shadow_mismatches > shadow_observations
            || (pending_events == 0 && pending_lag_ms != 0)
            || !signal_rollup_consistent
            || !audit_head_consistent
        {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        Ok(CutoverAuthoritySnapshot {
            scope: expected_scope.clone(),
            phase,
            writer,
            mirror_enabled,
            replay_paused,
            epoch,
            phase_epoch,
            audit_head: self.audit_head,
            rollback_ready,
            phase_started_at_ms,
            updated_at_ms,
            compatibility: CompatibilityAuthority {
                phase: singleton_phase,
                writer: singleton_writer,
                epoch: singleton_epoch,
                updated_at_ms: singleton_updated_at_ms,
            },
            slo: CutoverSloPolicy {
                shadow_window_ms,
                minimum_shadow_observations,
                max_pending_lag_ms,
                max_shadow_mismatches,
                max_dead_letter_events,
                max_contention_events,
                updated_at_ms: slo_updated_at_ms,
            },
            shadow: CutoverShadowHealth {
                window_started_at_ms: observation_window_started_at_ms,
                required_query_classes,
                covered_query_classes,
                observations: shadow_observations,
                mismatches: shadow_mismatches,
            },
            replay: CutoverReplayHealth {
                pending_events,
                dead_letter_events,
                pending_lag_ms,
            },
            signals: CutoverSignalWindow {
                authority_contention,
                replay_write_failures,
                replay_lost_ack,
            },
            maintenance_window_active: boolean(self.maintenance_window_active)?,
        })
    }
}

impl WriterStateRow {
    fn matches(
        &self,
        fence: &AuthorityFence,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<bool> {
        let updated_at_ms = wire(self.updated_at_ms)?;
        let occurred_at_ms = u64::try_from(occurred_at.get())
            .map_err(|_| CutoverAuthorityFailure::InvalidRequest)?;
        let phase = parse_phase(&self.phase)?;
        let writer = parse_writer(&self.writer)?;
        let epoch = wire(self.epoch)?;
        let phase_epoch = wire(self.phase_epoch)?;
        if !lower_sha256(&self.audit_head) || phase_epoch > epoch {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        Ok(self.tenant_id == fence.scope.tenant_id.to_string()
            && self.domain == fence.scope.domain.as_str()
            && writer == phase_writer(phase)
            && writer == fence.writer
            && epoch == fence.epoch
            && updated_at_ms <= occurred_at_ms)
    }

    fn matches_control(
        &self,
        scope: &CutoverScope,
        expected_epoch: u64,
        expected_audit_head: &str,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<bool> {
        let phase = parse_phase(&self.phase)?;
        let writer = parse_writer(&self.writer)?;
        let epoch = wire(self.epoch)?;
        let phase_epoch = wire(self.phase_epoch)?;
        let updated_at_ms = wire(self.updated_at_ms)?;
        let occurred_at_ms = u64::try_from(occurred_at.get())
            .map_err(|_| CutoverAuthorityFailure::InvalidRequest)?;
        if !lower_sha256(&self.audit_head) || phase_epoch > epoch {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        Ok(self.tenant_id == scope.tenant_id.to_string()
            && self.domain == scope.domain.as_str()
            && writer == phase_writer(phase)
            && epoch == expected_epoch
            && self.audit_head == expected_audit_head
            && updated_at_ms <= occurred_at_ms)
    }

    fn active_phase_epoch(
        &self,
        scope: &CutoverScope,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<Option<u64>> {
        let phase = parse_phase(&self.phase)?;
        let writer = parse_writer(&self.writer)?;
        let epoch = wire(self.epoch)?;
        let phase_epoch = wire(self.phase_epoch)?;
        let updated_at_ms = wire(self.updated_at_ms)?;
        let occurred_at_ms = u64::try_from(occurred_at.get())
            .map_err(|_| CutoverAuthorityFailure::InvalidRequest)?;
        if !lower_sha256(&self.audit_head) || phase_epoch > epoch {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        Ok((self.tenant_id == scope.tenant_id.to_string()
            && self.domain == scope.domain.as_str()
            && writer == phase_writer(phase)
            && updated_at_ms <= occurred_at_ms)
            .then_some(phase_epoch))
    }
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn wire(value: i64) -> CutoverAuthorityResult<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value <= MAX_WIRE_INTEGER)
        .ok_or(CutoverAuthorityFailure::Corrupt)
}

fn positive_wire(value: i64) -> CutoverAuthorityResult<u64> {
    wire(value).and_then(|value| {
        (value > 0)
            .then_some(value)
            .ok_or(CutoverAuthorityFailure::Corrupt)
    })
}

fn boolean(value: i64) -> CutoverAuthorityResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CutoverAuthorityFailure::Corrupt),
    }
}

fn parse_phase(value: &str) -> CutoverAuthorityResult<CutoverPhase> {
    match value {
        "legacy_authoritative" => Ok(CutoverPhase::LegacyAuthoritative),
        "shadow_read" => Ok(CutoverPhase::ShadowRead),
        "dual_write" => Ok(CutoverPhase::DualWrite),
        "d1_authoritative" => Ok(CutoverPhase::D1Authoritative),
        "rolled_back" => Ok(CutoverPhase::RolledBack),
        "finalized" => Ok(CutoverPhase::Finalized),
        _ => Err(CutoverAuthorityFailure::Corrupt),
    }
}

fn parse_writer(value: &str) -> CutoverAuthorityResult<DataAuthority> {
    match value {
        "legacy" => Ok(DataAuthority::Legacy),
        "d1" => Ok(DataAuthority::D1),
        _ => Err(CutoverAuthorityFailure::Corrupt),
    }
}

fn parse_compatibility_writer(
    phase: CutoverPhase,
    value: &str,
) -> CutoverAuthorityResult<DataAuthority> {
    if matches!(phase, CutoverPhase::DualWrite) && value == "dual_write" {
        // Released Workers treat this historical pair as mutation-disabled;
        // the legacy source remains the only effective writer.
        return Ok(DataAuthority::Legacy);
    }
    parse_writer(value)
}

const fn phase_writer(phase: CutoverPhase) -> DataAuthority {
    match phase {
        CutoverPhase::LegacyAuthoritative
        | CutoverPhase::ShadowRead
        | CutoverPhase::DualWrite
        | CutoverPhase::RolledBack => DataAuthority::Legacy,
        CutoverPhase::D1Authoritative | CutoverPhase::Finalized => DataAuthority::D1,
    }
}

const fn mirror_phase(phase: CutoverPhase) -> bool {
    matches!(
        phase,
        CutoverPhase::DualWrite | CutoverPhase::D1Authoritative | CutoverPhase::RolledBack
    )
}

const fn phase_code(phase: CutoverPhase) -> &'static str {
    match phase {
        CutoverPhase::LegacyAuthoritative => "legacy_authoritative",
        CutoverPhase::ShadowRead => "shadow_read",
        CutoverPhase::DualWrite => "dual_write",
        CutoverPhase::D1Authoritative => "d1_authoritative",
        CutoverPhase::RolledBack => "rolled_back",
        CutoverPhase::Finalized => "finalized",
    }
}

const fn writer_code(writer: DataAuthority) -> &'static str {
    match writer {
        DataAuthority::Legacy => "legacy",
        DataAuthority::D1 => "d1",
    }
}

fn digest_fields(domain: &str, fields: &[String]) -> String {
    let mut digest = Sha256::new();
    for field in std::iter::once(domain).chain(fields.iter().map(String::as_str)) {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let bytes = digest.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn transition_evidence_digest(
    evidence: CutoverEvidence,
    reconciliation_digest: Option<&str>,
) -> CutoverAuthorityResult<String> {
    if evidence.reconciliation_digest_present != reconciliation_digest.is_some()
        || reconciliation_digest.is_some_and(|digest| !lower_sha256(digest))
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    Ok(digest_fields(
        "frame-cutover-transition-evidence-v1",
        &[
            evidence.shadow_observation_ready.to_string(),
            evidence.reconciliation_clean.to_string(),
            evidence.rollback_rehearsed.to_string(),
            evidence.observation_window_complete.to_string(),
            evidence.reconciliation_digest_present.to_string(),
            evidence.legacy_fenced.to_string(),
            evidence.d1_fenced.to_string(),
            evidence.legacy_caught_up.to_string(),
            evidence.pending_events.to_string(),
            evidence.dead_letter_events.to_string(),
            evidence.shadow_mismatches.to_string(),
            reconciliation_digest.unwrap_or("").to_owned(),
        ],
    ))
}

#[allow(clippy::too_many_arguments)]
fn audit_digest(
    previous_hash: &str,
    scope: &CutoverScope,
    action: &str,
    from_phase: CutoverPhase,
    to_phase: CutoverPhase,
    from_epoch: u64,
    to_epoch: u64,
    operator_digest: &str,
    evidence_digest: &str,
    occurred_at: TimestampMillis,
) -> String {
    digest_fields(
        "frame-cutover-authority-audit-v1",
        &[
            previous_hash.to_owned(),
            scope.tenant_id.to_string(),
            scope.domain.as_str().to_owned(),
            action.to_owned(),
            phase_code(from_phase).to_owned(),
            phase_code(to_phase).to_owned(),
            from_epoch.to_string(),
            to_epoch.to_string(),
            operator_digest.to_owned(),
            evidence_digest.to_owned(),
            occurred_at.get().to_string(),
        ],
    )
}

fn state_from_snapshot(snapshot: &CutoverAuthoritySnapshot) -> CutoverState {
    CutoverState {
        scope: snapshot.scope.clone(),
        phase: snapshot.phase,
        writer: snapshot.writer,
        mirror_enabled: snapshot.mirror_enabled,
        replay_paused: snapshot.replay_paused,
        epoch: snapshot.epoch,
    }
}

fn validate_transition_command(
    snapshot: &CutoverAuthoritySnapshot,
    command: &ApprovedCutoverTransition,
) -> CutoverAuthorityResult<(CutoverState, String)> {
    if command.scope != snapshot.scope
        || command.expected_epoch > MAX_WIRE_INTEGER
        || !lower_sha256(&command.operator_digest)
        || matches!(
            command.target,
            CutoverPhase::LegacyAuthoritative | CutoverPhase::Finalized
        )
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    if command.expected_epoch != snapshot.epoch {
        return Err(CutoverAuthorityFailure::StaleAuthority);
    }
    if !matches!(command.target, CutoverPhase::RolledBack) && !snapshot.maintenance_window_active {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    if matches!(phase_writer(command.target), DataAuthority::Legacy)
        && matches!(snapshot.compatibility.writer, DataAuthority::D1)
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    if command.evidence.pending_events != snapshot.replay.pending_events
        || command.evidence.dead_letter_events != snapshot.replay.dead_letter_events
        || command.evidence.shadow_mismatches != snapshot.shadow.mismatches
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    let needs_forward_health = matches!(
        command.target,
        CutoverPhase::DualWrite | CutoverPhase::D1Authoritative
    );
    let needs_drained_replay = matches!(command.target, CutoverPhase::D1Authoritative);
    if needs_forward_health
        && (snapshot.replay_paused
            || command.evidence.observation_window_complete != snapshot.shadow.coverage_complete()
            || !snapshot.forward_health_is_clean(needs_drained_replay))
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    let evidence_digest =
        transition_evidence_digest(command.evidence, command.reconciliation_digest.as_deref())?;
    let mut proposed = state_from_snapshot(snapshot);
    proposed
        .transition(command.target, command.evidence)
        .map_err(|_| CutoverAuthorityFailure::InvalidRequest)?;
    if !proposed.invariants_hold() {
        return Err(CutoverAuthorityFailure::Corrupt);
    }
    Ok((proposed, evidence_digest))
}

fn validate_replay_control_command(
    snapshot: &CutoverAuthoritySnapshot,
    command: &ApprovedReplayControl,
) -> CutoverAuthorityResult<(CutoverState, String)> {
    if command.scope != snapshot.scope
        || command.expected_epoch > MAX_WIRE_INTEGER
        || !lower_sha256(&command.operator_digest)
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    if command.expected_epoch != snapshot.epoch {
        return Err(CutoverAuthorityFailure::StaleAuthority);
    }
    if matches!(command.action, ReplayControlAction::Resume) && !snapshot.maintenance_window_active
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    let mut proposed = state_from_snapshot(snapshot);
    proposed
        .set_replay_paused(command.action.paused())
        .map_err(|_| CutoverAuthorityFailure::InvalidRequest)?;
    if !proposed.invariants_hold() {
        return Err(CutoverAuthorityFailure::Corrupt);
    }
    let evidence_digest = digest_fields(
        "frame-cutover-replay-control-evidence-v1",
        &[
            command.action.as_str().to_owned(),
            command.action.paused().to_string(),
        ],
    );
    Ok((proposed, evidence_digest))
}

fn safe_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPERATION_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b':' | b'/' | b'@' | b'+' | b'-')
        })
}

fn safe_query_class(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn validate_shadow_observation(
    observation: &CutoverShadowObservation,
) -> CutoverAuthorityResult<()> {
    if observation.phase_epoch > MAX_WIRE_INTEGER
        || !safe_query_class(&observation.query_class)
        || !lower_sha256(&observation.observation_digest)
        || !lower_sha256(&observation.normalization_digest)
        || !lower_sha256(&observation.legacy_result_digest)
        || !lower_sha256(&observation.d1_result_digest)
    {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    Ok(())
}

fn number(value: u64) -> CutoverAuthorityResult<JsValue> {
    if value > MAX_WIRE_INTEGER {
        return Err(CutoverAuthorityFailure::InvalidRequest);
    }
    Ok(JsValue::from_f64(value as f64))
}

pub struct D1CutoverAuthorityRepository<'database> {
    database: &'database D1Database,
}

impl<'database> D1CutoverAuthorityRepository<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> CutoverAuthorityResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| CutoverAuthorityFailure::Unavailable)
    }

    async fn rows<T: for<'de> Deserialize<'de>>(
        &self,
        statement: D1PreparedStatement,
    ) -> CutoverAuthorityResult<Vec<T>> {
        let results = self
            .database
            .batch(vec![statement])
            .into_send()
            .await
            .map_err(|_| CutoverAuthorityFailure::Unavailable)?;
        if results.len() != 1 || !results[0].success() {
            return Err(CutoverAuthorityFailure::Unavailable);
        }
        results[0]
            .results::<serde_json::Value>()
            .map_err(|_| CutoverAuthorityFailure::Unavailable)?
            .into_iter()
            .map(|row| serde_json::from_value(row).map_err(|_| CutoverAuthorityFailure::Corrupt))
            .collect()
    }

    pub async fn snapshot(
        &self,
        scope: &CutoverScope,
        observed_at: TimestampMillis,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        let statement = self.statement(
            AUTHORITY_SNAPSHOT_SQL,
            &[
                JsValue::from_str(&scope.tenant_id.to_string()),
                JsValue::from_str(scope.domain.as_str()),
                JsValue::from_f64(observed_at.get() as f64),
            ],
        )?;
        let mut rows = self.rows::<AuthoritySnapshotRow>(statement).await?;
        if rows.len() > 1 {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        rows.pop()
            .ok_or(CutoverAuthorityFailure::NotFound)?
            .decode(scope, observed_at)
    }

    fn writer_assertion(
        &self,
        assertion_id: &str,
        fence: &AuthorityFence,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<D1PreparedStatement> {
        self.statement(
            WRITER_ASSERT_SQL,
            &[
                JsValue::from_str(assertion_id),
                JsValue::from_str(&fence.scope.tenant_id.to_string()),
                JsValue::from_str(fence.scope.domain.as_str()),
                JsValue::from_str(match fence.writer {
                    DataAuthority::Legacy => "legacy",
                    DataAuthority::D1 => "d1",
                }),
                number(fence.epoch)?,
                JsValue::from_f64(occurred_at.get() as f64),
            ],
        )
    }

    async fn writer_state(&self, scope: &CutoverScope) -> CutoverAuthorityResult<WriterStateRow> {
        let statement = self.statement(
            WRITER_STATE_SQL,
            &[
                JsValue::from_str(&scope.tenant_id.to_string()),
                JsValue::from_str(scope.domain.as_str()),
            ],
        )?;
        let mut rows = self.rows::<WriterStateRow>(statement).await?;
        if rows.len() > 1 {
            return Err(CutoverAuthorityFailure::Corrupt);
        }
        rows.pop().ok_or(CutoverAuthorityFailure::NotFound)
    }

    async fn classify_phase_event_failure(
        &self,
        scope: &CutoverScope,
        expected_phase_epoch: u64,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityFailure {
        match self.writer_state(scope).await {
            Ok(row) => match row.active_phase_epoch(scope, occurred_at) {
                Ok(Some(phase_epoch)) if phase_epoch == expected_phase_epoch => {
                    CutoverAuthorityFailure::MutationRejected
                }
                Ok(_) => CutoverAuthorityFailure::StaleAuthority,
                Err(CutoverAuthorityFailure::Corrupt) => CutoverAuthorityFailure::Corrupt,
                Err(_) => CutoverAuthorityFailure::Unavailable,
            },
            Err(CutoverAuthorityFailure::NotFound) => CutoverAuthorityFailure::StaleAuthority,
            Err(CutoverAuthorityFailure::Corrupt) => CutoverAuthorityFailure::Corrupt,
            Err(_) => CutoverAuthorityFailure::Unavailable,
        }
    }

    async fn record_current_signal(
        &self,
        scope: &CutoverScope,
        kind: CutoverSignalKind,
        occurred_at: TimestampMillis,
    ) {
        let Ok(row) = self.writer_state(scope).await else {
            return;
        };
        let Ok(Some(phase_epoch)) = row.active_phase_epoch(scope, occurred_at) else {
            return;
        };
        let _ = self
            .record_signal(scope, phase_epoch, kind, occurred_at)
            .await;
    }

    async fn execute_authority_batch(
        &self,
        statements: Vec<D1PreparedStatement>,
        scope: &CutoverScope,
        expected_epoch: u64,
        expected_audit_head: &str,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<()> {
        let expected_results = statements.len();
        let result = self.database.batch(statements).into_send().await;
        if result.as_ref().is_ok_and(|results| {
            results.len() == expected_results && results.iter().all(worker::D1Result::success)
        }) {
            return Ok(());
        }
        match self.writer_state(scope).await {
            Ok(row)
                if row.matches_control(
                    scope,
                    expected_epoch,
                    expected_audit_head,
                    occurred_at,
                )? =>
            {
                Err(CutoverAuthorityFailure::MutationRejected)
            }
            Ok(_) | Err(CutoverAuthorityFailure::NotFound) => {
                Err(CutoverAuthorityFailure::StaleAuthority)
            }
            Err(CutoverAuthorityFailure::Corrupt) => Err(CutoverAuthorityFailure::Corrupt),
            Err(_) => Err(CutoverAuthorityFailure::Unavailable),
        }
    }

    /// Applies an already authenticated and approved phase transition. The
    /// live health assertion, audit append, scoped state update, and
    /// postcondition execute in one D1 transaction.
    pub async fn transition(
        &self,
        command: &ApprovedCutoverTransition,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        let snapshot = self.snapshot(&command.scope, command.occurred_at).await?;
        let validated = validate_transition_command(&snapshot, command);
        if matches!(&validated, Err(CutoverAuthorityFailure::StaleAuthority)) {
            self.record_current_signal(
                &snapshot.scope,
                CutoverSignalKind::AuthorityContention,
                command.occurred_at,
            )
            .await;
        }
        let (proposed, evidence_digest) = validated?;
        let audit_hash = audit_digest(
            &snapshot.audit_head,
            &snapshot.scope,
            "transition",
            snapshot.phase,
            proposed.phase,
            snapshot.epoch,
            proposed.epoch,
            &command.operator_digest,
            &evidence_digest,
            command.occurred_at,
        );
        let gate_id = format!("{audit_hash}:transition");
        let postcondition_id = format!("{audit_hash}:postcondition");
        let tenant = snapshot.scope.tenant_id.to_string();
        let domain = snapshot.scope.domain.as_str();
        let occurred_at = JsValue::from_f64(command.occurred_at.get() as f64);
        let requires_maintenance = !matches!(proposed.phase, CutoverPhase::RolledBack);
        let requires_forward_health = matches!(
            proposed.phase,
            CutoverPhase::DualWrite | CutoverPhase::D1Authoritative
        );
        let requires_drained_replay = matches!(
            proposed.phase,
            CutoverPhase::D1Authoritative | CutoverPhase::RolledBack
        );
        let statements = vec![
            self.statement(
                TRANSITION_ASSERT_SQL,
                &[
                    JsValue::from_str(&gate_id),
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    number(snapshot.epoch)?,
                    JsValue::from_str(&snapshot.audit_head),
                    JsValue::from_str(phase_code(proposed.phase)),
                    occurred_at.clone(),
                    JsValue::from_bool(requires_maintenance),
                    JsValue::from_bool(requires_forward_health),
                    JsValue::from_bool(requires_drained_replay),
                ],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                &[
                    JsValue::from_str(&audit_hash),
                    JsValue::from_str(&snapshot.audit_head),
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    JsValue::from_str("transition"),
                    JsValue::from_str(phase_code(snapshot.phase)),
                    JsValue::from_str(phase_code(proposed.phase)),
                    number(snapshot.epoch)?,
                    number(proposed.epoch)?,
                    JsValue::from_str(&command.operator_digest),
                    JsValue::from_str(&evidence_digest),
                    occurred_at.clone(),
                ],
            )?,
            self.statement(
                SCOPE_TRANSITION_SQL,
                &[
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    JsValue::from_str(phase_code(snapshot.phase)),
                    number(snapshot.epoch)?,
                    JsValue::from_str(&snapshot.audit_head),
                    JsValue::from_str(phase_code(proposed.phase)),
                    JsValue::from_str(writer_code(proposed.writer)),
                    JsValue::from_bool(proposed.mirror_enabled),
                    JsValue::from_str(&audit_hash),
                    JsValue::from_bool(matches!(proposed.phase, CutoverPhase::D1Authoritative)),
                    occurred_at.clone(),
                ],
            )?,
            self.statement(
                STATE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&postcondition_id),
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    JsValue::from_str(phase_code(proposed.phase)),
                    JsValue::from_str(writer_code(proposed.writer)),
                    JsValue::from_bool(false),
                    number(proposed.epoch)?,
                    JsValue::from_str(&audit_hash),
                    occurred_at.clone(),
                    number(proposed.epoch)?,
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&gate_id)])?,
            self.statement(
                ASSERTION_CLEANUP_SQL,
                &[JsValue::from_str(&postcondition_id)],
            )?,
        ];
        let result = self
            .execute_authority_batch(
                statements,
                &snapshot.scope,
                snapshot.epoch,
                &snapshot.audit_head,
                command.occurred_at,
            )
            .await;
        if matches!(&result, Err(CutoverAuthorityFailure::StaleAuthority)) {
            self.record_current_signal(
                &snapshot.scope,
                CutoverSignalKind::AuthorityContention,
                command.occurred_at,
            )
            .await;
        }
        result?;
        self.snapshot(&command.scope, command.occurred_at).await
    }

    /// Pauses or resumes replay behind the same audited epoch fence used by
    /// phase transitions. Resume additionally requires a live maintenance
    /// window; pause remains available as an emergency control.
    pub async fn replay_control(
        &self,
        command: &ApprovedReplayControl,
    ) -> CutoverAuthorityResult<CutoverAuthoritySnapshot> {
        let snapshot = self.snapshot(&command.scope, command.occurred_at).await?;
        let validated = validate_replay_control_command(&snapshot, command);
        if matches!(&validated, Err(CutoverAuthorityFailure::StaleAuthority)) {
            self.record_current_signal(
                &snapshot.scope,
                CutoverSignalKind::AuthorityContention,
                command.occurred_at,
            )
            .await;
        }
        let (proposed, evidence_digest) = validated?;
        let action = command.action.as_str();
        let audit_hash = audit_digest(
            &snapshot.audit_head,
            &snapshot.scope,
            action,
            snapshot.phase,
            proposed.phase,
            snapshot.epoch,
            proposed.epoch,
            &command.operator_digest,
            &evidence_digest,
            command.occurred_at,
        );
        let gate_id = format!("{audit_hash}:{action}");
        let postcondition_id = format!("{audit_hash}:postcondition");
        let tenant = snapshot.scope.tenant_id.to_string();
        let domain = snapshot.scope.domain.as_str();
        let occurred_at = JsValue::from_f64(command.occurred_at.get() as f64);
        let statements = vec![
            self.statement(
                CONTROL_ASSERT_SQL,
                &[
                    JsValue::from_str(&gate_id),
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    number(snapshot.epoch)?,
                    JsValue::from_str(&snapshot.audit_head),
                    JsValue::from_str(action),
                    occurred_at.clone(),
                    JsValue::from_bool(matches!(command.action, ReplayControlAction::Resume)),
                ],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                &[
                    JsValue::from_str(&audit_hash),
                    JsValue::from_str(&snapshot.audit_head),
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    JsValue::from_str(action),
                    JsValue::from_str(phase_code(snapshot.phase)),
                    JsValue::from_str(phase_code(proposed.phase)),
                    number(snapshot.epoch)?,
                    number(proposed.epoch)?,
                    JsValue::from_str(&command.operator_digest),
                    JsValue::from_str(&evidence_digest),
                    occurred_at.clone(),
                ],
            )?,
            self.statement(
                SCOPE_CONTROL_SQL,
                &[
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    JsValue::from_str(phase_code(snapshot.phase)),
                    number(snapshot.epoch)?,
                    JsValue::from_str(&snapshot.audit_head),
                    JsValue::from_bool(proposed.replay_paused),
                    JsValue::from_str(&audit_hash),
                    occurred_at.clone(),
                ],
            )?,
            self.statement(
                STATE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&postcondition_id),
                    JsValue::from_str(&tenant),
                    JsValue::from_str(domain),
                    JsValue::from_str(phase_code(proposed.phase)),
                    JsValue::from_str(writer_code(proposed.writer)),
                    JsValue::from_bool(proposed.replay_paused),
                    number(proposed.epoch)?,
                    JsValue::from_str(&audit_hash),
                    occurred_at.clone(),
                    number(snapshot.phase_epoch)?,
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&gate_id)])?,
            self.statement(
                ASSERTION_CLEANUP_SQL,
                &[JsValue::from_str(&postcondition_id)],
            )?,
        ];
        let result = self
            .execute_authority_batch(
                statements,
                &snapshot.scope,
                snapshot.epoch,
                &snapshot.audit_head,
                command.occurred_at,
            )
            .await;
        if matches!(&result, Err(CutoverAuthorityFailure::StaleAuthority)) {
            self.record_current_signal(
                &snapshot.scope,
                CutoverSignalKind::AuthorityContention,
                command.occurred_at,
            )
            .await;
        }
        result?;
        self.snapshot(&command.scope, command.occurred_at).await
    }

    /// Executes application mutations in the same D1 transaction as the
    /// tenant/domain/writer/epoch assertion.
    pub async fn execute_fenced_batch(
        &self,
        operation_id: &str,
        fence: &AuthorityFence,
        occurred_at: TimestampMillis,
        mutations: Vec<D1PreparedStatement>,
    ) -> CutoverAuthorityResult<()> {
        self.execute_fenced_batch_results(operation_id, fence, occurred_at, mutations)
            .await
            .map(|_| ())
    }

    /// The same fenced transaction as [`Self::execute_fenced_batch`], while
    /// preserving only the caller mutation results for adapters that must
    /// validate exact row-count postconditions.
    pub async fn execute_fenced_batch_results(
        &self,
        operation_id: &str,
        fence: &AuthorityFence,
        occurred_at: TimestampMillis,
        mut mutations: Vec<D1PreparedStatement>,
    ) -> CutoverAuthorityResult<Vec<D1Result>> {
        if !safe_operation_id(operation_id)
            || mutations.is_empty()
            || mutations.len() > MAX_FENCED_MUTATIONS
        {
            return Err(CutoverAuthorityFailure::InvalidRequest);
        }
        let assertion_id = format!(
            "{}:writer",
            digest_fields(
                "frame-cutover-writer-assertion-v1",
                &[
                    operation_id.to_owned(),
                    fence.scope.tenant_id.to_string(),
                    fence.scope.domain.as_str().to_owned(),
                    writer_code(fence.writer).to_owned(),
                    fence.epoch.to_string(),
                ],
            )
        );
        let mut statements = Vec::with_capacity(mutations.len() + 2);
        statements.push(self.writer_assertion(&assertion_id, fence, occurred_at)?);
        statements.append(&mut mutations);
        statements
            .push(self.statement(ASSERTION_CLEANUP_SQL, &[JsValue::from_str(&assertion_id)])?);
        let expected_results = statements.len();
        let result = self.database.batch(statements).into_send().await;
        if result.as_ref().is_ok_and(|results| {
            results.len() == expected_results && results.iter().all(worker::D1Result::success)
        }) {
            let mut results = result.map_err(|_| CutoverAuthorityFailure::Unavailable)?;
            results.pop();
            results.remove(0);
            return Ok(results);
        }
        let result = match self.writer_state(&fence.scope).await {
            Ok(row) if row.matches(fence, occurred_at)? => {
                Err(CutoverAuthorityFailure::MutationRejected)
            }
            Ok(_) | Err(CutoverAuthorityFailure::NotFound) => {
                Err(CutoverAuthorityFailure::StaleAuthority)
            }
            Err(CutoverAuthorityFailure::Corrupt) => Err(CutoverAuthorityFailure::Corrupt),
            Err(_) => Err(CutoverAuthorityFailure::Unavailable),
        };
        if matches!(&result, Err(CutoverAuthorityFailure::StaleAuthority)) {
            self.record_current_signal(
                &fence.scope,
                CutoverSignalKind::AuthorityContention,
                occurred_at,
            )
            .await;
        }
        result
    }

    /// Stores only digests and a bounded classification. The database binds
    /// the observation to an approved query normalization and the stable phase
    /// epoch, so pause/resume controls preserve the window while transitions
    /// cannot reuse it.
    pub async fn record_shadow_observation(
        &self,
        observation: &CutoverShadowObservation,
    ) -> CutoverAuthorityResult<()> {
        validate_shadow_observation(observation)?;
        let statement = self.statement(
            SHADOW_OBSERVATION_INSERT_SQL,
            &[
                JsValue::from_str(&observation.observation_digest),
                JsValue::from_str(&observation.scope.tenant_id.to_string()),
                JsValue::from_str(observation.scope.domain.as_str()),
                number(observation.phase_epoch)?,
                JsValue::from_str(&observation.query_class),
                JsValue::from_str(&observation.normalization_digest),
                JsValue::from_str(&observation.legacy_result_digest),
                JsValue::from_str(&observation.d1_result_digest),
                JsValue::from_str(observation.classification.as_str()),
                JsValue::from_f64(observation.observed_at.get() as f64),
            ],
        )?;
        let results = self.database.batch(vec![statement]).into_send().await;
        if results
            .as_ref()
            .is_ok_and(|results| results.len() == 1 && results[0].success())
        {
            return Ok(());
        }
        Err(self
            .classify_phase_event_failure(
                &observation.scope,
                observation.phase_epoch,
                observation.observed_at,
            )
            .await)
    }

    pub async fn record_signal(
        &self,
        scope: &CutoverScope,
        expected_phase_epoch: u64,
        kind: CutoverSignalKind,
        occurred_at: TimestampMillis,
    ) -> CutoverAuthorityResult<()> {
        if expected_phase_epoch > MAX_WIRE_INTEGER {
            return Err(CutoverAuthorityFailure::InvalidRequest);
        }
        let statement = self.statement(
            SIGNAL_INSERT_SQL,
            &[
                JsValue::from_str(&scope.tenant_id.to_string()),
                JsValue::from_str(scope.domain.as_str()),
                number(expected_phase_epoch)?,
                JsValue::from_str(kind.as_str()),
                JsValue::from_f64(occurred_at.get() as f64),
            ],
        )?;
        let results = self.database.batch(vec![statement]).into_send().await;
        if results
            .as_ref()
            .is_ok_and(|results| results.len() == 1 && results[0].success())
        {
            return Ok(());
        }
        Err(self
            .classify_phase_event_failure(scope, expected_phase_epoch, occurred_at)
            .await)
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::{CutoverDomain, TenantId};

    use super::*;

    fn scope() -> CutoverScope {
        CutoverScope::new(
            TenantId::parse("00000000-0000-0000-0000-000000000017").expect("tenant"),
            CutoverDomain::parse("metadata").expect("domain"),
        )
    }

    fn row() -> AuthoritySnapshotRow {
        AuthoritySnapshotRow {
            tenant_id: "00000000-0000-0000-0000-000000000017".into(),
            domain: "metadata".into(),
            phase: "dual_write".into(),
            writer: "legacy".into(),
            mirror_enabled: 1,
            replay_paused: 0,
            epoch: 2,
            phase_epoch: 2,
            audit_head: "a".repeat(64),
            rollback_ready: 0,
            phase_started_at_ms: 100,
            updated_at_ms: 100,
            singleton_phase: "legacy_authoritative".into(),
            singleton_authority: "legacy".into(),
            singleton_epoch: 0,
            singleton_updated_at_ms: 0,
            shadow_window_ms: 1_000,
            minimum_shadow_observations: 2,
            max_pending_lag_ms: 500,
            max_shadow_mismatches: 0,
            max_dead_letter_events: 0,
            max_contention_events: 1,
            approved_by_digest: "b".repeat(64),
            slo_updated_at_ms: 90,
            observation_window_started_at_ms: 100,
            required_query_classes: 2,
            covered_query_classes: 2,
            shadow_observations: 4,
            shadow_mismatches: 0,
            pending_events: 0,
            dead_letter_events: 0,
            pending_lag_ms: 0,
            authority_contention_events: 1,
            replay_write_failures: 0,
            replay_lost_ack_events: 0,
            signal_rollup_consistent: 1,
            audit_head_consistent: 1,
            maintenance_window_active: 1,
        }
    }

    const fn d1_evidence() -> CutoverEvidence {
        CutoverEvidence {
            shadow_observation_ready: true,
            reconciliation_clean: true,
            rollback_rehearsed: true,
            observation_window_complete: true,
            reconciliation_digest_present: true,
            legacy_fenced: true,
            d1_fenced: false,
            legacy_caught_up: false,
            pending_events: 0,
            dead_letter_events: 0,
            shadow_mismatches: 0,
        }
    }

    #[test]
    fn snapshot_requires_one_writer_and_fresh_bounded_health() {
        let snapshot = row()
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("snapshot");
        assert_eq!(snapshot.phase, CutoverPhase::DualWrite);
        assert_eq!(snapshot.writer, DataAuthority::Legacy);
        assert!(snapshot.shadow.coverage_complete());
        assert!(snapshot.promotion_health_is_clean());
        assert!(snapshot.maintenance_window_active);

        let mut invalid = row();
        invalid.writer = "d1".into();
        assert_eq!(
            invalid.decode(&scope(), TimestampMillis::new(200).expect("time")),
            Err(CutoverAuthorityFailure::Corrupt)
        );
        let mut future = row();
        future.updated_at_ms = 201;
        assert_eq!(
            future.decode(&scope(), TimestampMillis::new(200).expect("time")),
            Err(CutoverAuthorityFailure::Corrupt)
        );
    }

    #[test]
    fn singleton_conflict_and_signals_block_promotion() {
        let mut incompatible = row();
        incompatible.singleton_phase = "d1_authoritative".into();
        incompatible.singleton_authority = "d1".into();
        let snapshot = incompatible
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("snapshot");
        assert!(snapshot.compatibility_conflict());
        assert!(!snapshot.promotion_health_is_clean());
        assert_eq!(
            snapshot.authorize_writer(DataAuthority::Legacy, 2),
            Err(CutoverAuthorityFailure::StaleAuthority)
        );

        let mut historical = row();
        historical.singleton_phase = "dual_write".into();
        historical.singleton_authority = "dual_write".into();
        let snapshot = historical
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("historical compatibility row");
        assert_eq!(snapshot.compatibility.writer, DataAuthority::Legacy);

        let mut failed = row();
        failed.replay_write_failures = 1;
        let snapshot = failed
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("snapshot");
        assert!(!snapshot.promotion_health_is_clean());
    }

    #[test]
    fn scoped_fence_rejects_other_writer_or_epoch() {
        let snapshot = row()
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("snapshot");
        assert!(snapshot.authorize_writer(DataAuthority::Legacy, 2).is_ok());
        assert_eq!(
            snapshot.authorize_writer(DataAuthority::D1, 2),
            Err(CutoverAuthorityFailure::StaleAuthority)
        );
        assert_eq!(
            snapshot.authorize_writer(DataAuthority::Legacy, 1),
            Err(CutoverAuthorityFailure::StaleAuthority)
        );
    }

    #[test]
    fn identifiers_and_scalar_decoders_fail_closed() {
        assert!(safe_operation_id("operation-1/v1"));
        assert!(!safe_operation_id("operation with spaces"));
        assert!(!safe_operation_id(&"x".repeat(MAX_OPERATION_ID_BYTES + 1)));
        assert_eq!(boolean(2), Err(CutoverAuthorityFailure::Corrupt));
        assert_eq!(wire(-1), Err(CutoverAuthorityFailure::Corrupt));
        assert!(!lower_sha256(&"A".repeat(64)));
        assert!(safe_query_class("video_list-v1"));
        assert!(!safe_query_class("Video List"));

        let mut invalid_phase_epoch = row();
        invalid_phase_epoch.phase_epoch = 3;
        assert_eq!(
            invalid_phase_epoch.decode(&scope(), TimestampMillis::new(200).expect("time")),
            Err(CutoverAuthorityFailure::Corrupt)
        );

        let observation = CutoverShadowObservation {
            scope: scope(),
            phase_epoch: 2,
            observation_digest: "1".repeat(64),
            query_class: "video_list".into(),
            normalization_digest: "2".repeat(64),
            legacy_result_digest: "3".repeat(64),
            d1_result_digest: "3".repeat(64),
            classification: ShadowClassification::Match,
            observed_at: TimestampMillis::new(200).expect("time"),
        };
        assert!(validate_shadow_observation(&observation).is_ok());
        let mut unsafe_observation = observation;
        unsafe_observation.legacy_result_digest = "raw-private-value".repeat(4);
        assert_eq!(
            validate_shadow_observation(&unsafe_observation),
            Err(CutoverAuthorityFailure::InvalidRequest)
        );
    }

    #[test]
    fn approved_transition_binds_live_health_and_evidence_digest() {
        let snapshot = row()
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("snapshot");
        let command = ApprovedCutoverTransition {
            scope: scope(),
            target: CutoverPhase::D1Authoritative,
            expected_epoch: 2,
            operator_digest: "c".repeat(64),
            evidence: d1_evidence(),
            reconciliation_digest: Some("d".repeat(64)),
            occurred_at: TimestampMillis::new(200).expect("time"),
        };
        let (proposed, evidence_digest) =
            validate_transition_command(&snapshot, &command).expect("approved transition");
        assert_eq!(proposed.phase, CutoverPhase::D1Authoritative);
        assert_eq!(proposed.writer, DataAuthority::D1);
        assert_eq!(proposed.epoch, 3);
        assert!(lower_sha256(&evidence_digest));

        let mut stale_evidence = command.clone();
        stale_evidence.evidence.pending_events = 1;
        assert_eq!(
            validate_transition_command(&snapshot, &stale_evidence),
            Err(CutoverAuthorityFailure::InvalidRequest)
        );
        let mut missing_digest = command;
        missing_digest.reconciliation_digest = None;
        assert_eq!(
            validate_transition_command(&snapshot, &missing_digest),
            Err(CutoverAuthorityFailure::InvalidRequest)
        );
    }

    #[test]
    fn replay_controls_and_hashes_are_epoch_bound() {
        let snapshot = row()
            .decode(&scope(), TimestampMillis::new(200).expect("time"))
            .expect("snapshot");
        let pause = ApprovedReplayControl {
            scope: scope(),
            action: ReplayControlAction::Pause,
            expected_epoch: 2,
            operator_digest: "c".repeat(64),
            occurred_at: TimestampMillis::new(200).expect("time"),
        };
        let (paused, evidence_digest) =
            validate_replay_control_command(&snapshot, &pause).expect("pause");
        assert!(paused.replay_paused);
        assert_eq!(paused.epoch, 3);
        let first = audit_digest(
            &snapshot.audit_head,
            &snapshot.scope,
            "pause",
            snapshot.phase,
            paused.phase,
            snapshot.epoch,
            paused.epoch,
            &pause.operator_digest,
            &evidence_digest,
            pause.occurred_at,
        );
        let second = audit_digest(
            &snapshot.audit_head,
            &snapshot.scope,
            "pause",
            snapshot.phase,
            paused.phase,
            snapshot.epoch,
            paused.epoch,
            &pause.operator_digest,
            &evidence_digest,
            pause.occurred_at,
        );
        assert_eq!(first, second);
        assert!(lower_sha256(&first));
        assert_ne!(
            digest_fields("test", &["ab".into(), "c".into()]),
            digest_fields("test", &["a".into(), "bc".into()])
        );
    }
}
