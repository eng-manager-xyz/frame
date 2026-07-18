PRAGMA foreign_keys = ON;

-- Lossless Cap user/account projection. Frame's retained columns stay the
-- primary read surface; nullable compatibility columns preserve Cap fields
-- that do not otherwise exist in the scaffold.
ALTER TABLE users ADD COLUMN legacy_last_name TEXT
  CHECK (legacy_last_name IS NULL OR length(legacy_last_name) <= 255);
ALTER TABLE users ADD COLUMN legacy_image_key TEXT
  CHECK (legacy_image_key IS NULL OR length(legacy_image_key) <= 255);
ALTER TABLE users ADD COLUMN legacy_onboarding_steps_json TEXT
  CHECK (
    legacy_onboarding_steps_json IS NULL OR (
      json_valid(legacy_onboarding_steps_json)
      AND json_type(legacy_onboarding_steps_json) = 'object'
      AND length(legacy_onboarding_steps_json) <= 4096
    )
  );
ALTER TABLE users ADD COLUMN legacy_onboarding_completed_at_ms INTEGER
  CHECK (
    legacy_onboarding_completed_at_ms IS NULL
    OR legacy_onboarding_completed_at_ms BETWEEN 0 AND 9007199254740991
  );
ALTER TABLE users ADD COLUMN legacy_stripe_customer_id TEXT
  CHECK (legacy_stripe_customer_id IS NULL OR length(legacy_stripe_customer_id) <= 255);
ALTER TABLE users ADD COLUMN legacy_stripe_subscription_id TEXT
  CHECK (
    legacy_stripe_subscription_id IS NULL
    OR length(legacy_stripe_subscription_id) <= 255
  );
ALTER TABLE users ADD COLUMN legacy_stripe_subscription_status TEXT
  CHECK (
    legacy_stripe_subscription_status IS NULL
    OR length(legacy_stripe_subscription_status) <= 255
  );
ALTER TABLE users ADD COLUMN legacy_user_account_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_user_account_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE users ADD COLUMN legacy_user_account_authority_version INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_user_account_authority_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE users ADD COLUMN legacy_user_account_last_operation_id TEXT
  CHECK (
    legacy_user_account_last_operation_id IS NULL
    OR length(legacy_user_account_last_operation_id) = 36
  );

-- Cap accepts empty/255-character organization names while the retained Frame
-- column requires 1..160. `legacy_user_account_name` is the lossless value;
-- D1 stores a safe retained placeholder only when that narrower check requires
-- it. The adapter and migration proof always read COALESCE(shadow, retained).
ALTER TABLE organizations ADD COLUMN legacy_user_account_name TEXT
  CHECK (
    legacy_user_account_name IS NULL
    OR length(legacy_user_account_name) <= 255
  );
ALTER TABLE organizations ADD COLUMN legacy_icon_key TEXT
  CHECK (legacy_icon_key IS NULL OR length(legacy_icon_key) <= 255);

-- Source NanoIDs are not reversible from their collision-safe UUIDv8 mapping.
-- Exact onboarding RPC results therefore retain the imported/generated source
-- identifier explicitly.
CREATE TABLE legacy_user_account_organization_ids_v1 (
  organization_id TEXT PRIMARY KEY NOT NULL
    REFERENCES organizations(id) ON DELETE CASCADE,
  legacy_organization_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_organization_id) = 15
    AND legacy_organization_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  recorded_at_ms INTEGER NOT NULL
    CHECK (recorded_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36)
);

CREATE TABLE legacy_user_account_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN (
    'legacy.user.name', 'legacy.user.complete_onboarding', 'legacy.user.update',
    'legacy.account.patch', 'legacy.account.sign_out_all',
    'legacy.devtool.demote_from_pro', 'legacy.devtool.promote_to_pro',
    'legacy.devtool.restart_onboarding'
  )),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'applied')),
  result_kind TEXT CHECK (
    result_kind IS NULL OR result_kind IN (
      'json_true', 'onboarding', 'rpc_void', 'server_action_void'
    )
  ),
  onboarding_step TEXT CHECK (
    onboarding_step IS NULL OR onboarding_step IN (
      'welcome', 'organizationSetup', 'customDomain', 'inviteTeam', 'skipToDashboard'
    )
  ),
  result_legacy_organization_id TEXT CHECK (
    result_legacy_organization_id IS NULL OR (
      length(result_legacy_organization_id) = 15
      AND result_legacy_organization_id
        NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  provider_effect TEXT CHECK (
    provider_effect IS NULL OR provider_effect IN (
      'not_requested', 'applied', 'best_effort_failed', 'best_effort_protected_gate'
    )
  ),
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER
    CHECK (completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (actor_id, action, idempotency_key_digest),
  CHECK (
    (state = 'pending' AND result_kind IS NULL AND completed_at_ms IS NULL)
    OR (state = 'applied' AND result_kind IS NOT NULL AND completed_at_ms IS NOT NULL)
  ),
  CHECK (
    (result_kind = 'onboarding' AND onboarding_step IS NOT NULL)
    OR (result_kind <> 'onboarding' AND onboarding_step IS NULL)
    OR result_kind IS NULL
  ),
  CHECK (
    (onboarding_step = 'organizationSetup' AND result_legacy_organization_id IS NOT NULL)
    OR (onboarding_step <> 'organizationSetup' AND result_legacy_organization_id IS NULL)
    OR onboarding_step IS NULL
  )
);
CREATE INDEX legacy_user_account_operations_time_v1
  ON legacy_user_account_operations_v1(created_at_ms, operation_id);

CREATE TABLE legacy_user_account_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_user_account_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
  result_kind TEXT NOT NULL CHECK (
    result_kind IN ('json_true', 'onboarding', 'rpc_void', 'server_action_void')
  ),
  onboarding_step TEXT CHECK (
    onboarding_step IS NULL OR onboarding_step IN (
      'welcome', 'organizationSetup', 'customDomain', 'inviteTeam', 'skipToDashboard'
    )
  ),
  result_legacy_organization_id TEXT,
  provider_effect TEXT NOT NULL CHECK (provider_effect IN (
    'not_requested', 'applied', 'best_effort_failed', 'best_effort_protected_gate'
  )),
  resulting_user_revision INTEGER NOT NULL
    CHECK (resulting_user_revision BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_user_account_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_user_account_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
  provider_effect TEXT NOT NULL CHECK (provider_effect IN (
    'not_requested', 'applied', 'best_effort_failed', 'best_effort_protected_gate'
  )),
  value_json TEXT NOT NULL CHECK (
    json_valid(value_json) AND json_type(value_json) = 'object'
    AND length(value_json) <= 1024
  ),
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_user_account_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_user_account_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
  principal_subject_digest TEXT NOT NULL CHECK (
    length(principal_subject_digest) = 64
    AND principal_subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64 AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome = 'allow'),
  occurred_at_ms INTEGER NOT NULL
    CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_user_account_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'authority', 'organization_access', 'organization_projection',
    'mutation', 'receipt', 'effect', 'audit', 'operation_complete',
    'durable_postcondition'
  )),
  expected_count INTEGER NOT NULL
    CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL
    CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_user_account_authority_assertion_v1
BEFORE INSERT ON legacy_user_account_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind = 'authority'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_authority_v1');
END;

CREATE TRIGGER legacy_user_account_access_assertion_v1
BEFORE INSERT ON legacy_user_account_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind = 'organization_access'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_forbidden_v1');
END;

CREATE TRIGGER legacy_user_account_projection_assertion_v1
BEFORE INSERT ON legacy_user_account_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind = 'organization_projection'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_projection_v1');
END;

CREATE TRIGGER legacy_user_account_mutation_assertion_v1
BEFORE INSERT ON legacy_user_account_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind NOT IN (
    'authority', 'organization_access', 'organization_projection'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_corrupt_v1');
END;

CREATE TRIGGER legacy_user_account_operation_transition_v1
BEFORE UPDATE ON legacy_user_account_operations_v1
WHEN NOT (
  OLD.state = 'pending' AND NEW.state = 'applied'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.action = NEW.action
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_operation_immutable_v1');
END;

CREATE TRIGGER legacy_user_account_operation_delete_v1
BEFORE DELETE ON legacy_user_account_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;

CREATE TRIGGER legacy_user_account_receipt_update_v1
BEFORE UPDATE ON legacy_user_account_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_user_account_receipt_delete_v1
BEFORE DELETE ON legacy_user_account_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_user_account_effect_update_v1
BEFORE UPDATE ON legacy_user_account_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_user_account_effect_delete_v1
BEFORE DELETE ON legacy_user_account_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_user_account_audit_update_v1
BEFORE UPDATE ON legacy_user_account_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_user_account_audit_delete_v1
BEFORE DELETE ON legacy_user_account_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_user_account_evidence_immutable_v1');
END;
