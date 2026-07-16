PRAGMA foreign_keys = ON;

-- The released authority_state singleton remains a fail-closed compatibility gate
-- for older Workers. New cutovers use these tenant/domain-scoped controls.
CREATE TABLE cutover_authority_scopes (
  tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 128),
  domain TEXT NOT NULL CHECK (
    length(domain) BETWEEN 1 AND 64
    AND substr(domain, 1, 1) GLOB '[a-z]'
    AND domain NOT GLOB '*[^a-z0-9_-]*'
  ),
  phase TEXT NOT NULL CHECK (phase IN (
    'legacy_authoritative', 'shadow_read', 'dual_write',
    'd1_authoritative', 'rolled_back', 'finalized'
  )),
  writer TEXT NOT NULL CHECK (writer IN ('legacy', 'd1')),
  mirror_enabled INTEGER NOT NULL CHECK (mirror_enabled IN (0, 1)),
  replay_paused INTEGER NOT NULL CHECK (replay_paused IN (0, 1)),
  epoch INTEGER NOT NULL CHECK (epoch BETWEEN 0 AND 9007199254740991),
  phase_epoch INTEGER NOT NULL DEFAULT 0 CHECK (
    phase_epoch BETWEEN 0 AND epoch
  ),
  audit_head TEXT NOT NULL CHECK (
    length(audit_head) = 64 AND audit_head NOT GLOB '*[^0-9a-f]*'
  ),
  rollback_ready INTEGER NOT NULL CHECK (rollback_ready IN (0, 1)),
  phase_started_at_ms INTEGER NOT NULL CHECK (
    phase_started_at_ms BETWEEN 0 AND updated_at_ms
  ),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (phase IN ('legacy_authoritative', 'shadow_read', 'dual_write', 'rolled_back')
      AND writer = 'legacy')
    OR (phase IN ('d1_authoritative', 'finalized') AND writer = 'd1')
  ),
  CHECK (mirror_enabled = (phase IN ('dual_write', 'd1_authoritative', 'rolled_back'))),
  CHECK (rollback_ready = (phase = 'd1_authoritative')),
  PRIMARY KEY (tenant_id, domain)
);

-- Fenced D1 batches insert one assertion row before the application mutation
-- and remove it in the same transaction. A false assertion aborts the complete
-- batch, closing the read-then-write race at an authority epoch boundary.
CREATE TABLE cutover_repository_assertions_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) BETWEEN 1 AND 192),
  satisfied INTEGER NOT NULL CHECK (satisfied = 1)
) WITHOUT ROWID;

CREATE TRIGGER cutover_repository_assertion_satisfied
BEFORE INSERT ON cutover_repository_assertions_v1
WHEN NEW.satisfied <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_cutover_authority_conflict_v1');
END;

CREATE TABLE cutover_authority_audit (
  audit_hash TEXT PRIMARY KEY NOT NULL CHECK (
    length(audit_hash) = 64 AND audit_hash NOT GLOB '*[^0-9a-f]*'
  ),
  previous_hash TEXT NOT NULL CHECK (
    length(previous_hash) = 64 AND previous_hash NOT GLOB '*[^0-9a-f]*'
  ),
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  action TEXT NOT NULL CHECK (action IN ('transition', 'pause', 'resume')),
  from_phase TEXT NOT NULL CHECK (from_phase IN (
    'legacy_authoritative', 'shadow_read', 'dual_write',
    'd1_authoritative', 'rolled_back', 'finalized'
  )),
  to_phase TEXT NOT NULL CHECK (to_phase IN (
    'legacy_authoritative', 'shadow_read', 'dual_write',
    'd1_authoritative', 'rolled_back', 'finalized'
  )),
  from_epoch INTEGER NOT NULL CHECK (from_epoch BETWEEN 0 AND 9007199254740990),
  to_epoch INTEGER NOT NULL CHECK (to_epoch = from_epoch + 1),
  operator_digest TEXT NOT NULL CHECK (
    length(operator_digest) = 64 AND operator_digest NOT GLOB '*[^0-9a-f]*'
  ),
  evidence_digest TEXT NOT NULL CHECK (
    length(evidence_digest) = 64 AND evidence_digest NOT GLOB '*[^0-9a-f]*'
  ),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (tenant_id, domain, to_epoch),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
);

CREATE TRIGGER cutover_authority_scope_initial_state
BEFORE INSERT ON cutover_authority_scopes
WHEN NOT (
  NEW.phase = 'legacy_authoritative'
  AND NEW.writer = 'legacy'
  AND NEW.mirror_enabled = 0
  AND NEW.replay_paused = 0
  AND NEW.epoch = 0
  AND NEW.phase_epoch = 0
  AND NEW.audit_head = lower(hex(zeroblob(32)))
  AND NEW.rollback_ready = 0
  AND NEW.phase_started_at_ms = NEW.updated_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'cutover authority scope must start at fenced legacy authority');
END;

CREATE TABLE cutover_change_events (
  event_id TEXT NOT NULL CHECK (length(event_id) BETWEEN 1 AND 128),
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 9007199254740991),
  authority_epoch INTEGER NOT NULL CHECK (authority_epoch BETWEEN 0 AND 9007199254740991),
  source_authority TEXT NOT NULL CHECK (source_authority IN ('legacy', 'd1')),
  event_digest TEXT NOT NULL CHECK (
    length(event_digest) = 64 AND event_digest NOT GLOB '*[^0-9a-f]*'
  ),
  payload_ciphertext TEXT NOT NULL CHECK (length(payload_ciphertext) > 0),
  state TEXT NOT NULL CHECK (state IN ('pending', 'applied', 'dead_letter')),
  reason_code TEXT CHECK (
    reason_code IS NULL OR (
      length(reason_code) BETWEEN 1 AND 128
      AND substr(reason_code, 1, 1) GLOB '[a-z]'
      AND reason_code NOT GLOB '*[^a-z0-9_-]*'
    )
  ),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  captured_at_ms INTEGER NOT NULL CHECK (captured_at_ms BETWEEN 0 AND 9007199254740991),
  applied_at_ms INTEGER CHECK (
    applied_at_ms IS NULL OR applied_at_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY (tenant_id, domain, event_id),
  UNIQUE (tenant_id, domain, sequence),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
);
CREATE INDEX cutover_change_events_replay_idx
  ON cutover_change_events(tenant_id, domain, state, sequence);

CREATE TRIGGER cutover_change_event_authority_fence
BEFORE INSERT ON cutover_change_events
WHEN NOT EXISTS (
  SELECT 1 FROM cutover_change_events AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.event_id = NEW.event_id
    AND existing.event_digest = NEW.event_digest
    AND existing.sequence = NEW.sequence
)
AND NOT EXISTS (
  SELECT 1 FROM cutover_authority_scopes AS authority
  WHERE authority.tenant_id = NEW.tenant_id
    AND authority.domain = NEW.domain
    AND authority.writer = NEW.source_authority
    AND authority.epoch = NEW.authority_epoch
    AND authority.mirror_enabled = 1
    AND authority.phase IN ('dual_write', 'd1_authoritative', 'rolled_back')
)
BEGIN
  SELECT RAISE(ABORT, 'cutover change event failed its writer fence');
END;

CREATE TRIGGER cutover_change_event_sequence_contiguous
BEFORE INSERT ON cutover_change_events
WHEN NOT EXISTS (
  SELECT 1 FROM cutover_change_events AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.event_id = NEW.event_id
    AND existing.event_digest = NEW.event_digest
    AND existing.sequence = NEW.sequence
)
AND NEW.sequence <> COALESCE((
  SELECT MAX(existing.sequence) + 1
  FROM cutover_change_events AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
), 1)
BEGIN
  SELECT RAISE(ABORT, 'cutover change event sequence is not contiguous');
END;

CREATE TABLE cutover_shadow_observations (
  observation_digest TEXT NOT NULL CHECK (
    length(observation_digest) = 64 AND observation_digest NOT GLOB '*[^0-9a-f]*'
  ),
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  phase_epoch INTEGER NOT NULL CHECK (phase_epoch BETWEEN 0 AND 9007199254740991),
  query_class TEXT NOT NULL CHECK (
    length(query_class) BETWEEN 1 AND 64
    AND substr(query_class, 1, 1) GLOB '[a-z]'
    AND query_class NOT GLOB '*[^a-z0-9_-]*'
  ),
  normalization_digest TEXT NOT NULL CHECK (
    length(normalization_digest) = 64 AND normalization_digest NOT GLOB '*[^0-9a-f]*'
  ),
  legacy_result_digest TEXT NOT NULL CHECK (
    length(legacy_result_digest) = 64 AND legacy_result_digest NOT GLOB '*[^0-9a-f]*'
  ),
  d1_result_digest TEXT NOT NULL CHECK (
    length(d1_result_digest) = 64 AND d1_result_digest NOT GLOB '*[^0-9a-f]*'
  ),
  classification TEXT NOT NULL CHECK (classification IN (
    'match', 'ordering_only', 'semantic_mismatch', 'missing', 'error'
  )),
  observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (tenant_id, domain, observation_digest),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
);
CREATE INDEX cutover_shadow_scope_time_idx
  ON cutover_shadow_observations(
    tenant_id, domain, phase_epoch, query_class, observed_at_ms
  );

CREATE TABLE cutover_shadow_query_requirements (
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  query_class TEXT NOT NULL CHECK (
    length(query_class) BETWEEN 1 AND 64
    AND substr(query_class, 1, 1) GLOB '[a-z]'
    AND query_class NOT GLOB '*[^a-z0-9_-]*'
  ),
  normalization_digest TEXT NOT NULL CHECK (
    length(normalization_digest) = 64 AND normalization_digest NOT GLOB '*[^0-9a-f]*'
  ),
  approved_by_digest TEXT NOT NULL CHECK (
    length(approved_by_digest) = 64 AND approved_by_digest NOT GLOB '*[^0-9a-f]*'
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (tenant_id, domain, query_class),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
) WITHOUT ROWID;

CREATE TABLE cutover_operational_signals (
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN (
    'authority_contention', 'replay_write_failure', 'replay_lost_ack'
  )),
  count INTEGER NOT NULL CHECK (count BETWEEN 0 AND 9007199254740991),
  last_at_ms INTEGER NOT NULL CHECK (last_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (tenant_id, domain, kind),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
) WITHOUT ROWID;

CREATE TABLE cutover_operational_signal_events (
  signal_id INTEGER PRIMARY KEY AUTOINCREMENT,
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  phase_epoch INTEGER NOT NULL CHECK (phase_epoch BETWEEN 0 AND 9007199254740991),
  kind TEXT NOT NULL CHECK (kind IN (
    'authority_contention', 'replay_write_failure', 'replay_lost_ack'
  )),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
);
CREATE INDEX cutover_operational_signal_events_scope_time_idx
  ON cutover_operational_signal_events(
    tenant_id, domain, phase_epoch, occurred_at_ms, kind
  );

CREATE TABLE cutover_slo_config (
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  shadow_window_ms INTEGER NOT NULL CHECK (shadow_window_ms BETWEEN 1 AND 2592000000),
  minimum_shadow_observations INTEGER NOT NULL
    CHECK (minimum_shadow_observations BETWEEN 1 AND 1000000),
  max_pending_lag_ms INTEGER NOT NULL CHECK (max_pending_lag_ms BETWEEN 1 AND 2592000000),
  max_shadow_mismatches INTEGER NOT NULL
    CHECK (max_shadow_mismatches BETWEEN 0 AND 9007199254740991),
  max_dead_letter_events INTEGER NOT NULL
    CHECK (max_dead_letter_events BETWEEN 0 AND 9007199254740991),
  max_contention_events INTEGER NOT NULL
    CHECK (max_contention_events BETWEEN 0 AND 9007199254740991),
  approved_by_digest TEXT NOT NULL CHECK (
    length(approved_by_digest) = 64 AND approved_by_digest NOT GLOB '*[^0-9a-f]*'
  ),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (tenant_id, domain),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
);

CREATE TABLE cutover_maintenance_windows (
  tenant_id TEXT NOT NULL,
  domain TEXT NOT NULL,
  starts_at_ms INTEGER NOT NULL CHECK (starts_at_ms BETWEEN 0 AND 9007199254740991),
  ends_at_ms INTEGER NOT NULL CHECK (
    ends_at_ms BETWEEN starts_at_ms AND 9007199254740991
  ),
  approved_by_digest TEXT NOT NULL CHECK (
    length(approved_by_digest) = 64 AND approved_by_digest NOT GLOB '*[^0-9a-f]*'
  ),
  PRIMARY KEY (tenant_id, domain, starts_at_ms),
  FOREIGN KEY (tenant_id, domain)
    REFERENCES cutover_authority_scopes(tenant_id, domain) ON DELETE RESTRICT
);

CREATE TRIGGER cutover_authority_audit_immutable_update
BEFORE UPDATE ON cutover_authority_audit
BEGIN
  SELECT RAISE(ABORT, 'cutover authority audit is immutable');
END;

CREATE TRIGGER cutover_authority_audit_immutable_delete
BEFORE DELETE ON cutover_authority_audit
BEGIN
  SELECT RAISE(ABORT, 'cutover authority audit is immutable');
END;

CREATE TRIGGER cutover_authority_scope_epoch_monotonic
BEFORE UPDATE ON cutover_authority_scopes
WHEN NEW.epoch <> OLD.epoch + 1
BEGIN
  SELECT RAISE(ABORT, 'cutover authority epoch must advance exactly once');
END;

CREATE TRIGGER cutover_authority_scope_identity_immutable
BEFORE UPDATE ON cutover_authority_scopes
WHEN NEW.tenant_id <> OLD.tenant_id OR NEW.domain <> OLD.domain
BEGIN
  SELECT RAISE(ABORT, 'cutover authority scope identity is immutable');
END;

CREATE TRIGGER cutover_authority_scope_immutable_delete
BEFORE DELETE ON cutover_authority_scopes
BEGIN
  SELECT RAISE(ABORT, 'cutover authority scope is immutable');
END;

CREATE TRIGGER cutover_authority_scope_phase_epoch_guard
BEFORE UPDATE ON cutover_authority_scopes
WHEN (NEW.phase <> OLD.phase AND NEW.phase_epoch <> NEW.epoch)
  OR (NEW.phase = OLD.phase AND NEW.phase_epoch <> OLD.phase_epoch)
BEGIN
  SELECT RAISE(ABORT, 'cutover authority phase epoch is invalid');
END;

CREATE TRIGGER cutover_authority_audit_transition_valid
BEFORE INSERT ON cutover_authority_audit
WHEN NOT (
  (NEW.action = 'transition' AND (
    (NEW.from_phase = 'legacy_authoritative' AND NEW.to_phase = 'shadow_read')
    OR (NEW.from_phase = 'shadow_read' AND NEW.to_phase = 'dual_write')
    OR (NEW.from_phase = 'dual_write' AND NEW.to_phase = 'd1_authoritative')
    OR (NEW.from_phase = 'd1_authoritative' AND NEW.to_phase IN ('rolled_back', 'finalized'))
    OR (NEW.from_phase = 'rolled_back' AND NEW.to_phase = 'dual_write')
  ))
  OR (
    NEW.action IN ('pause', 'resume')
    AND NEW.from_phase = NEW.to_phase
    AND NEW.from_phase IN ('shadow_read', 'dual_write', 'd1_authoritative', 'rolled_back')
  )
)
BEGIN
  SELECT RAISE(ABORT, 'cutover authority audit transition is invalid');
END;

CREATE TRIGGER cutover_authority_audit_binds_current_state
BEFORE INSERT ON cutover_authority_audit
WHEN NOT EXISTS (
  SELECT 1
  FROM cutover_authority_scopes AS current
  WHERE current.tenant_id = NEW.tenant_id
    AND current.domain = NEW.domain
    AND current.phase = NEW.from_phase
    AND current.epoch = NEW.from_epoch
    AND current.audit_head = NEW.previous_hash
    AND NEW.occurred_at_ms >= current.updated_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'cutover authority audit does not bind current state');
END;

CREATE TRIGGER cutover_authority_scope_requires_audit
BEFORE UPDATE ON cutover_authority_scopes
WHEN NOT EXISTS (
  SELECT 1
  FROM cutover_authority_audit AS audit
  WHERE audit.audit_hash = NEW.audit_head
    AND audit.previous_hash = OLD.audit_head
    AND audit.tenant_id = OLD.tenant_id
    AND audit.domain = OLD.domain
    AND audit.from_phase = OLD.phase
    AND audit.to_phase = NEW.phase
    AND audit.from_epoch = OLD.epoch
    AND audit.to_epoch = NEW.epoch
    AND audit.occurred_at_ms = NEW.updated_at_ms
    AND (
      (audit.action = 'transition' AND NEW.phase <> OLD.phase AND NEW.replay_paused = 0
        AND NEW.phase_started_at_ms = NEW.updated_at_ms
        AND NEW.phase_epoch = NEW.epoch)
      OR (
        audit.action = 'pause' AND NEW.phase = OLD.phase
        AND OLD.replay_paused = 0 AND NEW.replay_paused = 1
        AND NEW.phase_started_at_ms = OLD.phase_started_at_ms
        AND NEW.phase_epoch = OLD.phase_epoch
      )
      OR (
        audit.action = 'resume' AND NEW.phase = OLD.phase
        AND OLD.replay_paused = 1 AND NEW.replay_paused = 0
        AND NEW.phase_started_at_ms = OLD.phase_started_at_ms
        AND NEW.phase_epoch = OLD.phase_epoch
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'cutover authority update requires a bound audit record');
END;

CREATE TRIGGER cutover_maintenance_windows_nonoverlap
BEFORE INSERT ON cutover_maintenance_windows
WHEN EXISTS (
  SELECT 1
  FROM cutover_maintenance_windows AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND NOT (NEW.ends_at_ms < existing.starts_at_ms OR NEW.starts_at_ms > existing.ends_at_ms)
)
BEGIN
  SELECT RAISE(ABORT, 'cutover maintenance windows cannot overlap');
END;

CREATE TRIGGER cutover_change_event_initial_shape
BEFORE INSERT ON cutover_change_events
WHEN NEW.state <> 'pending'
  OR NEW.reason_code IS NOT NULL
  OR NEW.applied_at_ms IS NOT NULL
  OR NEW.captured_at_ms < NEW.occurred_at_ms
BEGIN
  SELECT RAISE(ABORT, 'cutover change event must begin pending');
END;

CREATE TRIGGER cutover_change_event_idempotency_conflict
BEFORE INSERT ON cutover_change_events
WHEN EXISTS (
  SELECT 1 FROM cutover_change_events AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.event_id = NEW.event_id
)
AND NOT EXISTS (
  SELECT 1 FROM cutover_change_events AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.event_id = NEW.event_id
    AND existing.sequence = NEW.sequence
    AND existing.authority_epoch = NEW.authority_epoch
    AND existing.source_authority = NEW.source_authority
    AND existing.event_digest = NEW.event_digest
    AND existing.payload_ciphertext = NEW.payload_ciphertext
    AND existing.state = NEW.state
    AND existing.reason_code IS NEW.reason_code
    AND existing.occurred_at_ms = NEW.occurred_at_ms
    AND existing.captured_at_ms = NEW.captured_at_ms
    AND existing.applied_at_ms IS NEW.applied_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'cutover change event idempotency conflict');
END;

CREATE TRIGGER cutover_change_event_update_guard
BEFORE UPDATE ON cutover_change_events
WHEN NEW.event_id <> OLD.event_id
  OR NEW.tenant_id <> OLD.tenant_id
  OR NEW.domain <> OLD.domain
  OR NEW.sequence <> OLD.sequence
  OR NEW.authority_epoch <> OLD.authority_epoch
  OR NEW.source_authority <> OLD.source_authority
  OR NEW.event_digest <> OLD.event_digest
  OR NEW.payload_ciphertext <> OLD.payload_ciphertext
  OR NEW.occurred_at_ms <> OLD.occurred_at_ms
  OR NEW.captured_at_ms <> OLD.captured_at_ms
  OR NOT (
    (OLD.state = 'pending' AND NEW.state = 'applied'
      AND NEW.reason_code IS NULL
      AND NEW.applied_at_ms IS NOT NULL
      AND NEW.applied_at_ms >= OLD.captured_at_ms)
    OR
    (OLD.state = 'pending' AND NEW.state = 'dead_letter'
      AND NEW.reason_code IS NOT NULL
      AND NEW.applied_at_ms IS NULL)
  )
BEGIN
  SELECT RAISE(ABORT, 'cutover change event transition is invalid');
END;

CREATE TRIGGER cutover_change_event_immutable_delete
BEFORE DELETE ON cutover_change_events
BEGIN
  SELECT RAISE(ABORT, 'cutover change event is immutable');
END;

CREATE TRIGGER cutover_shadow_observation_current_phase
BEFORE INSERT ON cutover_shadow_observations
WHEN NOT EXISTS (
  SELECT 1 FROM cutover_authority_scopes AS authority
  WHERE authority.tenant_id = NEW.tenant_id
    AND authority.domain = NEW.domain
    AND authority.phase IN ('shadow_read', 'dual_write', 'd1_authoritative', 'rolled_back')
    AND authority.phase_epoch = NEW.phase_epoch
    AND NEW.observed_at_ms >= authority.updated_at_ms
    AND EXISTS (
      SELECT 1 FROM cutover_shadow_query_requirements AS requirement
      WHERE requirement.tenant_id = NEW.tenant_id
        AND requirement.domain = NEW.domain
        AND requirement.query_class = NEW.query_class
        AND requirement.normalization_digest = NEW.normalization_digest
    )
)
BEGIN
  SELECT RAISE(ABORT, 'cutover shadow observation is stale or unavailable');
END;

CREATE TRIGGER cutover_shadow_observation_idempotency_conflict
BEFORE INSERT ON cutover_shadow_observations
WHEN EXISTS (
  SELECT 1 FROM cutover_shadow_observations AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.observation_digest = NEW.observation_digest
)
AND NOT EXISTS (
  SELECT 1 FROM cutover_shadow_observations AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.observation_digest = NEW.observation_digest
    AND existing.phase_epoch = NEW.phase_epoch
    AND existing.query_class = NEW.query_class
    AND existing.normalization_digest = NEW.normalization_digest
    AND existing.legacy_result_digest = NEW.legacy_result_digest
    AND existing.d1_result_digest = NEW.d1_result_digest
    AND existing.classification = NEW.classification
    AND existing.observed_at_ms = NEW.observed_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'cutover shadow observation idempotency conflict');
END;

CREATE TRIGGER cutover_shadow_observation_immutable_update
BEFORE UPDATE ON cutover_shadow_observations
BEGIN
  SELECT RAISE(ABORT, 'cutover shadow observation is immutable');
END;

CREATE TRIGGER cutover_shadow_observation_immutable_delete
BEFORE DELETE ON cutover_shadow_observations
BEGIN
  SELECT RAISE(ABORT, 'cutover shadow observation is immutable');
END;

CREATE TRIGGER cutover_shadow_query_requirement_immutable_update
BEFORE UPDATE ON cutover_shadow_query_requirements
BEGIN
  SELECT RAISE(ABORT, 'cutover shadow query requirement is immutable');
END;

CREATE TRIGGER cutover_shadow_query_requirement_immutable_delete
BEFORE DELETE ON cutover_shadow_query_requirements
BEGIN
  SELECT RAISE(ABORT, 'cutover shadow query requirement is immutable');
END;

CREATE TRIGGER cutover_operational_signal_current_phase
BEFORE INSERT ON cutover_operational_signal_events
WHEN NOT EXISTS (
  SELECT 1 FROM cutover_authority_scopes AS authority
  WHERE authority.tenant_id = NEW.tenant_id
    AND authority.domain = NEW.domain
    AND authority.phase_epoch = NEW.phase_epoch
    AND NEW.occurred_at_ms >= authority.updated_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'cutover operational signal predates authority state');
END;

CREATE TRIGGER cutover_operational_signal_event_immutable_update
BEFORE UPDATE ON cutover_operational_signal_events
BEGIN
  SELECT RAISE(ABORT, 'cutover operational signal event is immutable');
END;

CREATE TRIGGER cutover_operational_signal_event_immutable_delete
BEFORE DELETE ON cutover_operational_signal_events
BEGIN
  SELECT RAISE(ABORT, 'cutover operational signal event is immutable');
END;

CREATE TRIGGER cutover_operational_signal_rollup
AFTER INSERT ON cutover_operational_signal_events
BEGIN
  UPDATE cutover_operational_signals
  SET count = count + 1,
      last_at_ms = MAX(last_at_ms, NEW.occurred_at_ms)
  WHERE tenant_id = NEW.tenant_id
    AND domain = NEW.domain
    AND kind = NEW.kind;
  INSERT OR IGNORE INTO cutover_operational_signals(
    tenant_id, domain, kind, count, last_at_ms
  ) VALUES (NEW.tenant_id, NEW.domain, NEW.kind, 1, NEW.occurred_at_ms);
END;

CREATE TRIGGER cutover_operational_signal_rollup_insert_exact
BEFORE INSERT ON cutover_operational_signals
WHEN NOT EXISTS (
  SELECT 1 FROM cutover_operational_signals AS existing
  WHERE existing.tenant_id = NEW.tenant_id
    AND existing.domain = NEW.domain
    AND existing.kind = NEW.kind
)
AND (
  NEW.count <> (
    SELECT COUNT(*) FROM cutover_operational_signal_events AS event
    WHERE event.tenant_id = NEW.tenant_id
      AND event.domain = NEW.domain
      AND event.kind = NEW.kind
  )
  OR NEW.last_at_ms <> COALESCE((
    SELECT MAX(event.occurred_at_ms) FROM cutover_operational_signal_events AS event
    WHERE event.tenant_id = NEW.tenant_id
      AND event.domain = NEW.domain
      AND event.kind = NEW.kind
  ), -1)
)
BEGIN
  SELECT RAISE(ABORT, 'cutover operational signal rollup differs from events');
END;

CREATE TRIGGER cutover_operational_signal_rollup_update_exact
BEFORE UPDATE ON cutover_operational_signals
WHEN NEW.tenant_id <> OLD.tenant_id
  OR NEW.domain <> OLD.domain
  OR NEW.kind <> OLD.kind
  OR NEW.count <> (
    SELECT COUNT(*) FROM cutover_operational_signal_events AS event
    WHERE event.tenant_id = OLD.tenant_id
      AND event.domain = OLD.domain
      AND event.kind = OLD.kind
  )
  OR NEW.last_at_ms <> COALESCE((
    SELECT MAX(event.occurred_at_ms) FROM cutover_operational_signal_events AS event
    WHERE event.tenant_id = OLD.tenant_id
      AND event.domain = OLD.domain
      AND event.kind = OLD.kind
  ), -1)
BEGIN
  SELECT RAISE(ABORT, 'cutover operational signal rollup differs from events');
END;

CREATE TRIGGER cutover_operational_signal_rollup_immutable_delete
BEFORE DELETE ON cutover_operational_signals
BEGIN
  SELECT RAISE(ABORT, 'cutover operational signal rollup is immutable');
END;

CREATE TRIGGER cutover_slo_config_immutable_update
BEFORE UPDATE ON cutover_slo_config
BEGIN
  SELECT RAISE(ABORT, 'cutover SLO configuration is immutable');
END;

CREATE TRIGGER cutover_slo_config_immutable_delete
BEFORE DELETE ON cutover_slo_config
BEGIN
  SELECT RAISE(ABORT, 'cutover SLO configuration is immutable');
END;

CREATE TRIGGER cutover_maintenance_window_immutable_update
BEFORE UPDATE ON cutover_maintenance_windows
BEGIN
  SELECT RAISE(ABORT, 'cutover maintenance window is immutable');
END;

CREATE TRIGGER cutover_maintenance_window_immutable_delete
BEFORE DELETE ON cutover_maintenance_windows
BEGIN
  SELECT RAISE(ABORT, 'cutover maintenance window is immutable');
END;

CREATE TRIGGER authority_state_singleton_immutable_delete
BEFORE DELETE ON authority_state
BEGIN
  SELECT RAISE(ABORT, 'authority singleton is immutable');
END;

CREATE TRIGGER authority_state_single_writer_pair
BEFORE UPDATE ON authority_state
WHEN NOT (
  (NEW.phase IN ('legacy_authoritative', 'shadow_read', 'dual_write', 'rolled_back')
    AND NEW.authority = 'legacy')
  OR (NEW.phase IN ('d1_authoritative', 'finalized') AND NEW.authority = 'd1')
)
OR NEW.updated_at_ms < OLD.updated_at_ms
BEGIN
  SELECT RAISE(ABORT, 'authority singleton violates the single-writer invariant');
END;

CREATE TRIGGER cutover_authority_scope_final_is_terminal
BEFORE UPDATE ON cutover_authority_scopes
WHEN OLD.phase = 'finalized'
BEGIN
  SELECT RAISE(ABORT, 'finalized cutover authority cannot transition');
END;
