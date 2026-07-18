PRAGMA foreign_keys = ON;

-- Cap developer applications are actor-owned and use source-shaped values
-- (`development`/`production`, 255-character names, nullable logos). The
-- retained Frame developer tables are organization-scoped and intentionally
-- have a different contract, so this expand slice keeps the compatibility
-- authority explicit instead of weakening the retained-table triggers.
CREATE TABLE legacy_developer_apps_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  legacy_app_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_app_id) = 15
    AND legacy_app_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 255 AND name = trim(name)),
  environment TEXT NOT NULL CHECK (environment IN ('development', 'production')),
  logo_url TEXT CHECK (logo_url IS NULL OR length(logo_url) <= 1024),
  deleted_at_ms INTEGER CHECK (
    deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  authority_version INTEGER NOT NULL DEFAULT 0
    CHECK (authority_version BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36)
);
CREATE INDEX legacy_developer_apps_owner_live_v1
  ON legacy_developer_apps_v1(owner_id, deleted_at_ms, id);

CREATE TABLE legacy_developer_app_domains_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  legacy_domain_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_domain_id) = 15
    AND legacy_domain_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  app_id TEXT NOT NULL REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  origin TEXT NOT NULL CHECK (
    length(origin) BETWEEN 1 AND 253
    AND origin = lower(origin)
    AND (origin LIKE 'https://%' OR origin LIKE 'http://%')
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36),
  UNIQUE (app_id, origin)
);
CREATE INDEX legacy_developer_domains_app_v1
  ON legacy_developer_app_domains_v1(app_id, id);

CREATE TABLE legacy_developer_api_keys_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  legacy_key_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_key_id) = 15
    AND legacy_key_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  app_id TEXT NOT NULL REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  key_kind TEXT NOT NULL CHECK (key_kind IN ('public', 'secret')),
  key_prefix TEXT NOT NULL CHECK (
    length(key_prefix) = 12
    AND (
      (key_kind = 'public' AND key_prefix LIKE 'cpk\_%' ESCAPE '\')
      OR (key_kind = 'secret' AND key_prefix LIKE 'csk\_%' ESCAPE '\')
    )
  ),
  key_digest TEXT NOT NULL UNIQUE CHECK (
    length(key_digest) = 64 AND key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  encrypted_key TEXT NOT NULL CHECK (length(encrypted_key) BETWEEN 1 AND 16384),
  revoked_at_ms INTEGER CHECK (
    revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36)
);
CREATE UNIQUE INDEX legacy_developer_active_key_kind_v1
  ON legacy_developer_api_keys_v1(app_id, key_kind) WHERE revoked_at_ms IS NULL;
CREATE INDEX legacy_developer_keys_app_v1
  ON legacy_developer_api_keys_v1(app_id, revoked_at_ms, id);

CREATE TABLE legacy_developer_videos_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  legacy_video_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_video_id) = 15
    AND legacy_video_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  app_id TEXT NOT NULL REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  deleted_at_ms INTEGER CHECK (
    deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36)
);
CREATE INDEX legacy_developer_videos_app_live_v1
  ON legacy_developer_videos_v1(app_id, deleted_at_ms, id);

CREATE TABLE legacy_developer_credit_accounts_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  legacy_credit_account_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_credit_account_id) = 15
    AND legacy_credit_account_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  app_id TEXT NOT NULL UNIQUE REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  balance_microcredits INTEGER NOT NULL DEFAULT 0
    CHECK (balance_microcredits BETWEEN 0 AND 9007199254740991),
  auto_top_up_enabled INTEGER NOT NULL DEFAULT 0 CHECK (auto_top_up_enabled IN (0, 1)),
  auto_top_up_threshold_microcredits INTEGER NOT NULL DEFAULT 0
    CHECK (auto_top_up_threshold_microcredits BETWEEN 0 AND 9007199254740991),
  auto_top_up_amount_cents INTEGER NOT NULL DEFAULT 0
    CHECK (auto_top_up_amount_cents BETWEEN 0 AND 100000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36)
);

CREATE TABLE legacy_developer_action_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN (
    'legacy.developer.create_app', 'legacy.developer.update_app',
    'legacy.developer.delete_app', 'legacy.developer.add_domain',
    'legacy.developer.remove_domain', 'legacy.developer.regenerate_keys',
    'legacy.developer.delete_video', 'legacy.developer.update_auto_top_up'
  )),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  ),
  UNIQUE (actor_id, action, idempotency_key_digest)
);
CREATE INDEX legacy_developer_action_operations_actor_time_v1
  ON legacy_developer_action_operations_v1(actor_id, created_at_ms DESC);

CREATE TRIGGER legacy_developer_action_operations_transition_v1
BEFORE UPDATE ON legacy_developer_action_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.action = NEW.action
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_operation_immutable_v1');
END;

CREATE TRIGGER legacy_developer_action_operations_delete_v1
BEFORE DELETE ON legacy_developer_action_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_operation_immutable_v1');
END;

CREATE TABLE legacy_developer_action_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_developer_action_operations_v1(operation_id) ON DELETE RESTRICT,
  result_kind TEXT NOT NULL CHECK (result_kind IN (
    'app_created', 'app_updated', 'app_deleted', 'domain_added',
    'domain_delete_attempted', 'keys_regenerated',
    'video_delete_attempted', 'auto_top_up_updated'
  )),
  app_id TEXT REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  legacy_app_id TEXT CHECK (legacy_app_id IS NULL OR length(legacy_app_id) = 15),
  final_name TEXT CHECK (final_name IS NULL OR length(final_name) BETWEEN 1 AND 255),
  final_environment TEXT CHECK (
    final_environment IS NULL OR final_environment IN ('development', 'production')
  ),
  final_logo_url TEXT CHECK (final_logo_url IS NULL OR length(final_logo_url) <= 1024),
  update_statement_executed INTEGER CHECK (
    update_statement_executed IS NULL OR update_statement_executed IN (0, 1)
  ),
  deleted_at_ms INTEGER CHECK (
    deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991
  ),
  revoked_active_key_count INTEGER CHECK (
    revoked_active_key_count IS NULL
    OR revoked_active_key_count BETWEEN 0 AND 4294967295
  ),
  active_key_count_after INTEGER CHECK (
    active_key_count_after IS NULL OR active_key_count_after BETWEEN 0 AND 2
  ),
  -- A valid but absent/cross-app target is part of the exact zero-row success
  -- receipt, so target identifiers deliberately are not foreign keys here.
  domain_id TEXT CHECK (domain_id IS NULL OR length(domain_id) = 36),
  legacy_domain_id TEXT CHECK (
    legacy_domain_id IS NULL OR (
      length(legacy_domain_id) = 15
      AND legacy_domain_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  stored_origin TEXT CHECK (stored_origin IS NULL OR length(stored_origin) BETWEEN 1 AND 253),
  matched_rows INTEGER CHECK (matched_rows IS NULL OR matched_rows IN (0, 1)),
  video_id TEXT CHECK (video_id IS NULL OR length(video_id) = 36),
  account_present INTEGER CHECK (account_present IS NULL OR account_present IN (0, 1)),
  auto_top_up_enabled INTEGER CHECK (
    auto_top_up_enabled IS NULL OR auto_top_up_enabled IN (0, 1)
  ),
  auto_top_up_threshold_microcredits INTEGER CHECK (
    auto_top_up_threshold_microcredits IS NULL
    OR auto_top_up_threshold_microcredits BETWEEN 0 AND 9007199254740991
  ),
  auto_top_up_amount_cents INTEGER CHECK (
    auto_top_up_amount_cents IS NULL OR auto_top_up_amount_cents BETWEEN 0 AND 100000
  ),
  credit_account_id TEXT
    REFERENCES legacy_developer_credit_accounts_v1(id) ON DELETE RESTRICT,
  public_key_id TEXT REFERENCES legacy_developer_api_keys_v1(id) ON DELETE RESTRICT,
  secret_key_id TEXT REFERENCES legacy_developer_api_keys_v1(id) ON DELETE RESTRICT,
  sealed_key_replay TEXT CHECK (
    sealed_key_replay IS NULL OR length(sealed_key_replay) BETWEEN 1 AND 16384
  ),
  replay_binding TEXT CHECK (
    replay_binding IS NULL
    OR (length(replay_binding) = 64 AND replay_binding NOT GLOB '*[^0-9a-f]*')
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (result_kind = 'app_created' AND app_id IS NOT NULL AND legacy_app_id IS NOT NULL
      AND final_name IS NOT NULL AND final_environment IS NOT NULL
      AND active_key_count_after = 2 AND credit_account_id IS NOT NULL
      AND account_present = 1 AND auto_top_up_enabled = 0
      AND auto_top_up_threshold_microcredits = 0 AND auto_top_up_amount_cents = 0
      AND public_key_id IS NOT NULL AND secret_key_id IS NOT NULL
      AND sealed_key_replay IS NOT NULL AND replay_binding IS NOT NULL)
    OR (result_kind = 'app_updated' AND app_id IS NOT NULL
      AND final_name IS NOT NULL AND final_environment IS NOT NULL
      AND update_statement_executed IS NOT NULL)
    OR (result_kind = 'app_deleted' AND app_id IS NOT NULL
      AND deleted_at_ms IS NOT NULL AND revoked_active_key_count IS NOT NULL
      AND active_key_count_after = 0)
    OR (result_kind = 'domain_added' AND app_id IS NOT NULL
      AND domain_id IS NOT NULL AND legacy_domain_id IS NOT NULL
      AND stored_origin IS NOT NULL)
    OR (result_kind = 'domain_delete_attempted' AND app_id IS NOT NULL
      AND domain_id IS NOT NULL AND matched_rows IS NOT NULL)
    OR (result_kind = 'keys_regenerated' AND app_id IS NOT NULL
      AND revoked_active_key_count IS NOT NULL AND active_key_count_after = 2
      AND public_key_id IS NOT NULL AND secret_key_id IS NOT NULL
      AND sealed_key_replay IS NOT NULL AND replay_binding IS NOT NULL)
    OR (result_kind = 'video_delete_attempted' AND app_id IS NOT NULL
      AND video_id IS NOT NULL AND matched_rows IS NOT NULL
      AND ((matched_rows = 0 AND deleted_at_ms IS NULL)
        OR (matched_rows = 1 AND deleted_at_ms IS NOT NULL)))
    OR (result_kind = 'auto_top_up_updated' AND app_id IS NOT NULL
      AND account_present IS NOT NULL
      AND ((account_present = 0 AND auto_top_up_enabled IS NULL
        AND auto_top_up_threshold_microcredits IS NULL AND auto_top_up_amount_cents IS NULL)
        OR (account_present = 1 AND auto_top_up_enabled IS NOT NULL
          AND auto_top_up_threshold_microcredits IS NOT NULL
          AND auto_top_up_amount_cents IS NOT NULL)))
  )
);

CREATE TABLE legacy_developer_action_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_developer_action_operations_v1(operation_id) ON DELETE RESTRICT,
  revalidate_developer_dashboard INTEGER NOT NULL
    CHECK (revalidate_developer_dashboard IN (0, 1)),
  revalidation_path TEXT CHECK (
    (revalidate_developer_dashboard = 0 AND revalidation_path IS NULL)
    OR (revalidate_developer_dashboard = 1 AND revalidation_path = '/dashboard/developers')
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_developer_action_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_developer_action_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64 AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome = 'allow'),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_developer_action_proof_consumptions_v1 (
  mutation_grant_id TEXT PRIMARY KEY NOT NULL CHECK (length(mutation_grant_id) = 36),
  session_id TEXT NOT NULL CHECK (length(session_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  related_operation_id TEXT CHECK (
    related_operation_id IS NULL OR length(related_operation_id) = 36
  ),
  action TEXT NOT NULL,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (
    outcome IN ('applied', 'replay', 'conflict', 'in_flight', 'rejected')
  ),
  consumed_at_ms INTEGER NOT NULL CHECK (consumed_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX legacy_developer_action_proofs_operation_v1
  ON legacy_developer_action_proof_consumptions_v1(related_operation_id, consumed_at_ms);

CREATE TABLE legacy_developer_action_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'browser_grant', 'grant_consumed', 'app_authority', 'app_mutated',
    'key_rows_mutated', 'domain_mutated', 'video_mutated', 'account_mutated',
    'postcondition', 'receipt_inserted', 'effect_inserted', 'audit_inserted',
    'proof_journaled', 'operation_complete', 'durable_receipt'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_developer_action_authority_assertion_v1
BEFORE INSERT ON legacy_developer_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN ('browser_grant', 'grant_consumed', 'app_authority')
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_authority_v1');
END;

CREATE TRIGGER legacy_developer_action_conflict_assertion_v1
BEFORE INSERT ON legacy_developer_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'app_mutated', 'key_rows_mutated', 'domain_mutated', 'video_mutated',
    'account_mutated', 'postcondition'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_conflict_v1');
END;

CREATE TRIGGER legacy_developer_action_corrupt_assertion_v1
BEFORE INSERT ON legacy_developer_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'receipt_inserted', 'effect_inserted', 'audit_inserted', 'proof_journaled',
    'operation_complete', 'durable_receipt'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_corrupt_v1');
END;

CREATE TRIGGER legacy_developer_action_receipts_update_v1
BEFORE UPDATE ON legacy_developer_action_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_receipts_delete_v1
BEFORE DELETE ON legacy_developer_action_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_effects_update_v1
BEFORE UPDATE ON legacy_developer_action_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_effects_delete_v1
BEFORE DELETE ON legacy_developer_action_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_audit_update_v1
BEFORE UPDATE ON legacy_developer_action_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_audit_delete_v1
BEFORE DELETE ON legacy_developer_action_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_proof_update_v1
BEFORE UPDATE ON legacy_developer_action_proof_consumptions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_proof_immutable_v1');
END;
CREATE TRIGGER legacy_developer_action_proof_delete_v1
BEFORE DELETE ON legacy_developer_action_proof_consumptions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_proof_immutable_v1');
END;
