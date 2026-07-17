PRAGMA foreign_keys = ON;

-- Browser-direct authenticated actions are claimed before any product row is
-- changed. The unique request key makes a retry replay the stored receipt,
-- while request_digest rejects key reuse with different input.
CREATE TABLE authenticated_web_action_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 96),
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 64),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'complete')),
  response_json TEXT CHECK (
    response_json IS NULL OR (json_valid(response_json) AND length(response_json) <= 8192)
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND response_json IS NULL AND completed_at_ms IS NULL)
    OR (state = 'complete' AND response_json IS NOT NULL AND completed_at_ms IS NOT NULL)
  ),
  UNIQUE (organization_id, user_id, action, idempotency_key)
);
CREATE INDEX authenticated_web_action_operations_v1_time_idx
  ON authenticated_web_action_operations_v1(organization_id, created_at_ms DESC);

-- Provider-neutral intent/effect audit for actions whose external execution is
-- a separately protected workflow. It deliberately stores bounded product
-- input, never session, CSRF, provider credential, or secret material.
CREATE TABLE authenticated_web_action_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES authenticated_web_action_operations_v1(operation_id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 96),
  effect_state TEXT NOT NULL CHECK (
    effect_state IN ('applied', 'pending_protected_execution')
  ),
  value_json TEXT NOT NULL CHECK (json_valid(value_json) AND length(value_json) <= 2048),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX authenticated_web_action_effects_v1_org_time_idx
  ON authenticated_web_action_effects_v1(organization_id, created_at_ms DESC);

-- A stale organization revision, changed membership role/revision, failed
-- changed-row postcondition, or missing one-use auth grant violates this CHECK
-- and aborts the complete D1 batch. Rows are deleted at the end of a successful
-- transaction, so this table never becomes a second authority ledger.
CREATE TABLE authenticated_web_action_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'organization_revision', 'selection_authority', 'membership_authority', 'mutation_grant',
    'product_effect', 'action_effect', 'organization_update',
    'operation_complete', 'grant_consumed'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);
