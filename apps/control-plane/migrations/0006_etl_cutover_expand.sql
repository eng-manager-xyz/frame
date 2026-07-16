PRAGMA foreign_keys = ON;

CREATE TABLE etl_runs (
  id TEXT PRIMARY KEY NOT NULL,
  source_revision TEXT NOT NULL CHECK (length(source_revision) BETWEEN 1 AND 128),
  source_snapshot_at_ms INTEGER NOT NULL CHECK (source_snapshot_at_ms BETWEEN 0 AND 9007199254740991),
  state TEXT NOT NULL CHECK (state IN ('planned', 'running', 'reconciling', 'matched', 'mismatched', 'failed')),
  started_at_ms INTEGER CHECK (started_at_ms IS NULL OR started_at_ms BETWEEN 0 AND 9007199254740991),
  finished_at_ms INTEGER CHECK (finished_at_ms IS NULL OR finished_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE etl_table_manifests (
  run_id TEXT NOT NULL REFERENCES etl_runs(id) ON DELETE CASCADE,
  table_name TEXT NOT NULL CHECK (length(table_name) BETWEEN 1 AND 64),
  expected_rows INTEGER NOT NULL CHECK (expected_rows BETWEEN 0 AND 9007199254740991),
  expected_bytes INTEGER CHECK (expected_bytes IS NULL OR expected_bytes BETWEEN 0 AND 9007199254740991),
  expected_checksum TEXT NOT NULL CHECK (length(expected_checksum) = 64),
  transform_version INTEGER NOT NULL CHECK (transform_version > 0),
  PRIMARY KEY (run_id, table_name)
);

CREATE TABLE etl_checkpoints (
  run_id TEXT NOT NULL REFERENCES etl_runs(id) ON DELETE CASCADE,
  table_name TEXT NOT NULL,
  checkpoint_ciphertext TEXT NOT NULL,
  processed_rows INTEGER NOT NULL CHECK (processed_rows BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  PRIMARY KEY (run_id, table_name),
  FOREIGN KEY (run_id, table_name) REFERENCES etl_table_manifests(run_id, table_name) ON DELETE CASCADE
);

CREATE TABLE etl_reconciliation_results (
  run_id TEXT NOT NULL REFERENCES etl_runs(id) ON DELETE CASCADE,
  table_name TEXT NOT NULL,
  expected_rows INTEGER NOT NULL CHECK (expected_rows BETWEEN 0 AND 9007199254740991),
  actual_rows INTEGER NOT NULL CHECK (actual_rows BETWEEN 0 AND 9007199254740991),
  expected_bytes INTEGER NOT NULL CHECK (expected_bytes BETWEEN 0 AND 9007199254740991),
  actual_bytes INTEGER NOT NULL CHECK (actual_bytes BETWEEN 0 AND 9007199254740991),
  source_checksum TEXT NOT NULL CHECK (length(source_checksum) = 64),
  target_checksum TEXT NOT NULL CHECK (length(target_checksum) = 64),
  relationship_mismatches INTEGER NOT NULL DEFAULT 0 CHECK (relationship_mismatches >= 0),
  semantic_mismatches INTEGER NOT NULL DEFAULT 0 CHECK (semantic_mismatches >= 0),
  reconciled_at_ms INTEGER NOT NULL CHECK (reconciled_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (run_id, table_name),
  FOREIGN KEY (run_id, table_name) REFERENCES etl_table_manifests(run_id, table_name) ON DELETE CASCADE
);

CREATE TABLE authority_state (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  phase TEXT NOT NULL CHECK (phase IN ('legacy_authoritative', 'shadow_read', 'dual_write', 'd1_authoritative', 'rolled_back', 'finalized')),
  authority TEXT NOT NULL CHECK (authority IN ('legacy', 'dual_write', 'd1')),
  epoch INTEGER NOT NULL CHECK (epoch >= 0 AND epoch <= 9007199254740991),
  reconciliation_run_id TEXT REFERENCES etl_runs(id) ON DELETE RESTRICT,
  rollback_rehearsed_at_ms INTEGER
    CHECK (rollback_rehearsed_at_ms IS NULL OR rollback_rehearsed_at_ms BETWEEN 0 AND 9007199254740991),
  observation_started_at_ms INTEGER
    CHECK (observation_started_at_ms IS NULL OR observation_started_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991)
);
INSERT INTO authority_state(singleton, phase, authority, epoch, updated_at_ms)
VALUES (1, 'legacy_authoritative', 'legacy', 0, 0);

CREATE TABLE authority_transition_audit (
  id TEXT PRIMARY KEY NOT NULL,
  from_phase TEXT NOT NULL,
  to_phase TEXT NOT NULL,
  from_epoch INTEGER NOT NULL CHECK (from_epoch >= 0),
  to_epoch INTEGER NOT NULL CHECK (to_epoch = from_epoch + 1),
  approved_by_digest TEXT NOT NULL CHECK (length(approved_by_digest) = 64),
  evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE dual_write_outcomes (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE,
  command_id TEXT NOT NULL,
  command_type TEXT NOT NULL CHECK (length(command_type) BETWEEN 1 AND 64),
  legacy_outcome_digest TEXT NOT NULL CHECK (length(legacy_outcome_digest) = 64),
  d1_outcome_digest TEXT NOT NULL CHECK (length(d1_outcome_digest) = 64),
  classification TEXT NOT NULL CHECK (classification IN ('match', 'retryable_mismatch', 'terminal_mismatch')),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  resolved_at_ms INTEGER CHECK (resolved_at_ms IS NULL OR resolved_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (command_id, command_type)
);
CREATE INDEX dual_write_outcomes_unresolved_idx
  ON dual_write_outcomes(classification, occurred_at_ms) WHERE resolved_at_ms IS NULL;

CREATE TABLE shadow_read_diffs (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE,
  query_class TEXT NOT NULL CHECK (length(query_class) BETWEEN 1 AND 64),
  legacy_result_digest TEXT NOT NULL CHECK (length(legacy_result_digest) = 64),
  d1_result_digest TEXT NOT NULL CHECK (length(d1_result_digest) = 64),
  classification TEXT NOT NULL CHECK (classification IN ('match', 'ordering_only', 'semantic_mismatch', 'missing', 'error')),
  correlation_id TEXT NOT NULL,
  observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX shadow_read_diffs_class_time_idx ON shadow_read_diffs(classification, observed_at_ms);

CREATE TRIGGER authority_state_epoch_monotonic
BEFORE UPDATE ON authority_state
WHEN NEW.epoch <> OLD.epoch + 1
BEGIN
  SELECT RAISE(ABORT, 'authority epoch must advance exactly once');
END;

CREATE TRIGGER authority_state_final_is_terminal
BEFORE UPDATE ON authority_state
WHEN OLD.phase = 'finalized'
BEGIN
  SELECT RAISE(ABORT, 'finalized authority cannot transition');
END;
