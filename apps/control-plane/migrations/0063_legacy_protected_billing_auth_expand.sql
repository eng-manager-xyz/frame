PRAGMA foreign_keys = ON;

-- Request handlers may create only immutable intents. A terminal response is
-- released after independent human approval (where required) and independent
-- provider execution evidence have both been recorded.
CREATE TABLE legacy_protected_billing_auth_receipts_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL CHECK (length(receipt_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (
    length(source_operation_id) = 23 AND source_operation_id LIKE 'cap-v1-%'
  ),
  operation_kind TEXT NOT NULL CHECK (
    operation_kind IN ('route','server_action','workflow')
  ),
  method TEXT NOT NULL CHECK (
    method IN ('GET','POST','OPTIONS','ACTION','WORKFLOW')
  ),
  surface_path TEXT NOT NULL CHECK (length(surface_path) BETWEEN 1 AND 512),
  auth_class TEXT NOT NULL CHECK (auth_class IN (
    'anonymous','public_or_flow_token','session','session_or_api_key',
    'admin_session','signed_webhook'
  )),
  authority_class TEXT NOT NULL CHECK (authority_class IN (
    'public_flow','active_session','developer_app_owner',
    'messenger_admin_video','signed_stripe_webhook'
  )),
  provider_kind TEXT NOT NULL CHECK (length(provider_kind) BETWEEN 1 AND 96),
  human_approval_required INTEGER NOT NULL CHECK (human_approval_required IN (0,1)),
  provider_execution_required INTEGER NOT NULL DEFAULT 1
    CHECK (provider_execution_required = 1),
  principal_digest TEXT NOT NULL CHECK (
    length(principal_digest) = 64 AND principal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  actor_id TEXT CHECK (actor_id IS NULL OR length(actor_id) BETWEEN 1 AND 255),
  credential_kind TEXT NOT NULL CHECK (credential_kind IN (
    'none','public_flow','session_token','api_key','signed_endpoint'
  )),
  credential_subject_id TEXT CHECK (
    credential_subject_id IS NULL OR length(credential_subject_id) BETWEEN 1 AND 255
  ),
  credential_key_version INTEGER CHECK (
    credential_key_version IS NULL OR credential_key_version BETWEEN 1 AND 65535
  ),
  credential_digest TEXT CHECK (
    credential_digest IS NULL OR (
      length(credential_digest) = 64
      AND credential_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  sealed_request_ref TEXT CHECK (
    sealed_request_ref IS NULL OR (
      length(sealed_request_ref) = 85
      AND substr(sealed_request_ref,1,21) = 'frame-pba-request-v1:'
      AND substr(sealed_request_ref,22) NOT GLOB '*[^0-9a-f]*'
    )
  ),
  sealed_request_digest TEXT CHECK (
    sealed_request_digest IS NULL OR (
      length(sealed_request_digest) = 64
      AND sealed_request_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  target_id TEXT CHECK (target_id IS NULL OR length(target_id) BETWEEN 1 AND 512),
  replay_key_digest TEXT NOT NULL CHECK (
    length(replay_key_digest) = 64 AND replay_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  replay_origin TEXT NOT NULL CHECK (
    replay_origin IN ('caller','natural','generated','nonce')
  ),
  idempotency_mode TEXT NOT NULL CHECK (
    idempotency_mode IN ('required','optional','forbidden')
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  redacted_request_json TEXT NOT NULL CHECK (
    json_valid(redacted_request_json)
    AND json_type(redacted_request_json) = 'object'
    AND length(redacted_request_json) <= 1048576
  ),
  state TEXT NOT NULL CHECK (state IN (
    'awaiting_human_approval','awaiting_provider_evidence',
    'verified','rejected','dead_letter'
  )),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  UNIQUE (source_operation_id, principal_digest, replay_key_digest),
  CHECK (
    (human_approval_required = 1 AND state = 'awaiting_human_approval'
      AND completed_at_ms IS NULL)
    OR (human_approval_required = 0 AND state = 'awaiting_provider_evidence'
      AND completed_at_ms IS NULL)
    OR (state IN ('verified','rejected','dead_letter')
      AND completed_at_ms IS NOT NULL)
  ),
  CHECK (
    (
      auth_class = 'anonymous'
      AND actor_id IS NULL
      AND credential_kind = 'public_flow'
      AND credential_subject_id IS NULL
      AND credential_key_version IS NULL
      AND credential_digest IS NOT NULL
    ) OR (
      auth_class = 'public_or_flow_token'
      AND actor_id IS NULL
      AND (
        (
          credential_kind = 'none'
          AND credential_subject_id IS NULL
          AND credential_key_version IS NULL
          AND credential_digest IS NULL
        ) OR (
          credential_kind = 'public_flow'
          AND credential_subject_id IS NULL
          AND credential_key_version IS NULL
          AND credential_digest IS NOT NULL
        )
      )
    ) OR (
      auth_class IN ('session','admin_session')
      AND actor_id IS NOT NULL
      AND credential_kind = 'session_token'
      AND length(credential_subject_id) = 36
      AND credential_key_version IS NOT NULL
      AND credential_digest IS NOT NULL
    ) OR (
      auth_class = 'session_or_api_key'
      AND actor_id IS NOT NULL
      AND (
        (
          credential_kind = 'session_token'
          AND length(credential_subject_id) = 36
          AND credential_key_version IS NOT NULL
          AND credential_digest IS NOT NULL
        ) OR (
          credential_kind = 'api_key'
          AND credential_subject_id IS NOT NULL
          AND credential_key_version IS NULL
          AND credential_digest IS NOT NULL
        )
      )
    ) OR (
      auth_class = 'signed_webhook'
      AND actor_id IS NULL
      AND credential_kind = 'signed_endpoint'
      AND credential_subject_id = 'stripe-webhook.endpoint.v1'
      AND credential_key_version IS NULL
      AND credential_digest IS NOT NULL
    )
  ),
  CHECK (
    (
      source_operation_id IN (
        'cap-v1-46bda1c18ffba076','cap-v1-82a39c991fae1050'
      )
      AND sealed_request_ref IS NOT NULL
      AND sealed_request_digest IS NOT NULL
    ) OR (
      source_operation_id NOT IN (
        'cap-v1-46bda1c18ffba076','cap-v1-82a39c991fae1050'
      )
      AND sealed_request_ref IS NULL
      AND sealed_request_digest IS NULL
    )
  )
);
CREATE INDEX legacy_protected_billing_auth_receipts_target_v1
  ON legacy_protected_billing_auth_receipts_v1(
    target_id,created_at_ms,receipt_id
  );

-- Released Cap routes did not consistently send idempotency keys. Their
-- internal generated namespace gets one atomic request-digest claim per
-- client-bound principal. Pending work remains reachable indefinitely; a
-- terminal claim may roll over only after the short retention below. Caller
-- keys and provider-natural identifiers never enter this table.
CREATE TABLE legacy_protected_billing_auth_generated_replay_claims_v1 (
  source_operation_id TEXT NOT NULL CHECK (
    length(source_operation_id) = 23 AND source_operation_id LIKE 'cap-v1-%'
  ),
  principal_digest TEXT NOT NULL CHECK (
    length(principal_digest) = 64 AND principal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  receipt_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_protected_billing_auth_receipts_v1(receipt_id) ON DELETE RESTRICT,
  claimed_at_ms INTEGER NOT NULL CHECK (
    claimed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY(source_operation_id,principal_digest,request_digest)
);

CREATE TRIGGER legacy_protected_billing_auth_generated_receipt_claim_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_receipts_v1
WHEN NEW.replay_origin = 'generated' AND EXISTS (
  SELECT 1
  FROM legacy_protected_billing_auth_generated_replay_claims_v1 claim
  JOIN legacy_protected_billing_auth_receipts_v1 prior
    ON prior.receipt_id = claim.receipt_id
  WHERE claim.source_operation_id = NEW.source_operation_id
    AND claim.principal_digest = NEW.principal_digest
    AND claim.request_digest = NEW.request_digest
    AND (
      prior.state IN ('awaiting_human_approval','awaiting_provider_evidence')
      OR prior.completed_at_ms IS NULL
      OR prior.completed_at_ms > NEW.created_at_ms - 900000
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_generated_replay_claimed_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_generated_claim_insert_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_generated_replay_claims_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_billing_auth_receipts_v1 receipt
  WHERE receipt.receipt_id = NEW.receipt_id
    AND receipt.source_operation_id = NEW.source_operation_id
    AND receipt.principal_digest = NEW.principal_digest
    AND receipt.request_digest = NEW.request_digest
    AND receipt.replay_origin = 'generated'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_generated_replay_invalid_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_generated_claim_update_gate_v1
BEFORE UPDATE ON legacy_protected_billing_auth_generated_replay_claims_v1
WHEN NOT (
  OLD.source_operation_id = NEW.source_operation_id
  AND OLD.principal_digest = NEW.principal_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.receipt_id <> NEW.receipt_id
  AND NEW.claimed_at_ms >= OLD.claimed_at_ms
  AND EXISTS (
    SELECT 1 FROM legacy_protected_billing_auth_receipts_v1 prior
    WHERE prior.receipt_id = OLD.receipt_id
      AND prior.state IN ('verified','rejected','dead_letter')
      AND prior.completed_at_ms IS NOT NULL
      AND prior.completed_at_ms <= NEW.claimed_at_ms - 900000
  )
  AND EXISTS (
    SELECT 1 FROM legacy_protected_billing_auth_receipts_v1 replacement
    WHERE replacement.receipt_id = NEW.receipt_id
      AND replacement.source_operation_id = NEW.source_operation_id
      AND replacement.principal_digest = NEW.principal_digest
      AND replacement.request_digest = NEW.request_digest
      AND replacement.replay_origin = 'generated'
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_generated_replay_immutable_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_generated_claim_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_generated_replay_claims_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_generated_replay_immutable_v1');
END;

-- A single relational projection is shared by replay and provider-evidence
-- gates. It binds the credential discriminator to the exact durable session
-- or API-key identity and re-evaluates actor/resource authority from current
-- D1 state. Callers must separately compare authority_expires_at_ms with the
-- time of the operation being authorized.
CREATE VIEW legacy_protected_billing_auth_live_authority_v1 AS
SELECT
  receipt.receipt_id,
  CASE
    WHEN receipt.authority_class IN ('public_flow','signed_stripe_webhook')
      THEN 9007199254740991
    WHEN receipt.credential_kind = 'session_token' THEN (
      SELECT MIN(session.idle_expires_at_ms,session.absolute_expires_at_ms)
      FROM auth_sessions_v2 session
      JOIN auth_identities_v2 identity ON identity.user_id = session.user_id
      WHERE session.id = receipt.credential_subject_id
        AND session.user_id = receipt.actor_id
        AND session.token_key_version = receipt.credential_key_version
        AND session.token_digest = receipt.credential_digest
        AND session.state = 'active'
        AND session.revoked_at_ms IS NULL
        AND session.session_version = identity.session_version
    )
    WHEN receipt.credential_kind = 'api_key' THEN (
      SELECT COALESCE(key.expires_at_ms,9007199254740991)
      FROM auth_api_keys key
      WHERE key.id = receipt.credential_subject_id
        AND key.user_id = receipt.actor_id
        AND key.key_digest = receipt.credential_digest
        AND key.revoked_at_ms IS NULL
    )
  END AS authority_expires_at_ms
FROM legacy_protected_billing_auth_receipts_v1 receipt
WHERE
  (
    receipt.authority_class = 'public_flow'
    AND receipt.credential_kind IN ('none','public_flow')
  )
  OR (
    receipt.authority_class = 'signed_stripe_webhook'
    AND receipt.credential_kind = 'signed_endpoint'
    AND receipt.credential_subject_id = 'stripe-webhook.endpoint.v1'
  )
  OR (
    receipt.authority_class = 'active_session'
    AND receipt.credential_kind IN ('session_token','api_key')
    AND EXISTS (
      SELECT 1 FROM users actor
      WHERE actor.id = receipt.actor_id
        AND actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
    )
  )
  OR (
    receipt.authority_class = 'developer_app_owner'
    AND receipt.credential_kind IN ('session_token','api_key')
    AND EXISTS (
      SELECT 1
      FROM users actor
      JOIN developer_apps app ON app.owner_user_id = actor.id
      JOIN developer_credit_accounts account ON account.app_id = app.id
      WHERE actor.id = receipt.actor_id
        AND actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
        AND app.id = receipt.target_id
        AND app.status = 'active'
        AND app.deleted_at_ms IS NULL
    )
  )
  OR (
    receipt.authority_class = 'messenger_admin_video'
    AND receipt.credential_kind = 'session_token'
    AND EXISTS (
      SELECT 1
      FROM users actor
      JOIN videos video ON video.id = receipt.target_id
      WHERE actor.id = receipt.actor_id
        AND actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
        AND actor.email = 'richie@cap.so' COLLATE NOCASE
        AND video.state <> 'deleted'
        AND video.deleted_at_ms IS NULL
    )
  );

-- The request adapter performs an early authority read to return a useful
-- 401, but this trigger is the actual authorization fence. It repeats the
-- live D1 authority decision inside the same transaction that inserts the
-- receipt, outbox, approval request, and (for server actions) consumes the
-- one-use browser mutation grant. Revocation or ownership changes between
-- the early read and this insert therefore abort the whole batch.
CREATE TRIGGER legacy_protected_billing_auth_receipt_authority_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_receipts_v1
WHEN NOT (
  (
    NEW.authority_class = 'public_flow'
    AND NEW.credential_kind IN ('none','public_flow')
  )
  OR (
    NEW.authority_class = 'signed_stripe_webhook'
    AND NEW.credential_kind = 'signed_endpoint'
    AND NEW.credential_subject_id = 'stripe-webhook.endpoint.v1'
  )
  OR (
    NEW.authority_class = 'active_session'
    AND EXISTS (
      SELECT 1 FROM users actor
      WHERE actor.id = NEW.actor_id
        AND actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
        AND (
          (
            NEW.credential_kind = 'session_token'
            AND EXISTS (
              SELECT 1
              FROM auth_sessions_v2 session
              JOIN auth_identities_v2 identity ON identity.user_id = session.user_id
              WHERE session.id = NEW.credential_subject_id
                AND session.user_id = actor.id
                AND session.token_key_version = NEW.credential_key_version
                AND session.token_digest = NEW.credential_digest
                AND session.state = 'active'
                AND session.revoked_at_ms IS NULL
                AND session.session_version = identity.session_version
                AND session.idle_expires_at_ms > NEW.created_at_ms
                AND session.absolute_expires_at_ms > NEW.created_at_ms
            )
          ) OR (
            NEW.credential_kind = 'api_key'
            AND NEW.credential_key_version IS NULL
            AND EXISTS (
            SELECT 1 FROM auth_api_keys key
            WHERE key.id = NEW.credential_subject_id
              AND key.user_id = actor.id
              AND key.key_digest = NEW.credential_digest
              AND key.revoked_at_ms IS NULL
              AND (
                key.expires_at_ms IS NULL
                OR key.expires_at_ms > NEW.created_at_ms
              )
            )
          )
        )
    )
  )
  OR (
    NEW.authority_class = 'developer_app_owner'
    AND EXISTS (
      SELECT 1
      FROM users actor
      JOIN developer_apps app ON app.owner_user_id = actor.id
      JOIN developer_credit_accounts account ON account.app_id = app.id
      WHERE actor.id = NEW.actor_id
        AND actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
        AND (
          (
            NEW.credential_kind = 'session_token'
            AND EXISTS (
              SELECT 1
              FROM auth_sessions_v2 session
              JOIN auth_identities_v2 identity ON identity.user_id = session.user_id
              WHERE session.id = NEW.credential_subject_id
                AND session.user_id = actor.id
                AND session.token_key_version = NEW.credential_key_version
                AND session.token_digest = NEW.credential_digest
                AND session.state = 'active'
                AND session.revoked_at_ms IS NULL
                AND session.session_version = identity.session_version
                AND session.idle_expires_at_ms > NEW.created_at_ms
                AND session.absolute_expires_at_ms > NEW.created_at_ms
            )
          ) OR (
            NEW.credential_kind = 'api_key'
            AND NEW.credential_key_version IS NULL
            AND EXISTS (
              SELECT 1 FROM auth_api_keys key
              WHERE key.id = NEW.credential_subject_id
                AND key.user_id = actor.id
                AND key.key_digest = NEW.credential_digest
                AND key.revoked_at_ms IS NULL
                AND (
                  key.expires_at_ms IS NULL
                  OR key.expires_at_ms > NEW.created_at_ms
                )
            )
          )
        )
        AND app.id = NEW.target_id
        AND app.status = 'active'
        AND app.deleted_at_ms IS NULL
    )
  )
  OR (
    NEW.authority_class = 'messenger_admin_video'
    AND NEW.credential_kind = 'session_token'
    AND EXISTS (
      SELECT 1
      FROM users actor
      JOIN auth_sessions_v2 session ON session.user_id = actor.id
      JOIN auth_identities_v2 identity ON identity.user_id = session.user_id
      JOIN videos video ON video.id = NEW.target_id
      WHERE actor.id = NEW.actor_id
        AND actor.status = 'active'
        AND actor.deleted_at_ms IS NULL
        AND actor.email = 'richie@cap.so' COLLATE NOCASE
        AND session.id = NEW.credential_subject_id
        AND session.token_key_version = NEW.credential_key_version
        AND session.token_digest = NEW.credential_digest
        AND session.state = 'active'
        AND session.revoked_at_ms IS NULL
        AND session.session_version = identity.session_version
        AND session.idle_expires_at_ms > NEW.created_at_ms
        AND session.absolute_expires_at_ms > NEW.created_at_ms
        AND video.state <> 'deleted'
        AND video.deleted_at_ms IS NULL
    )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_authority_stale_v1');
END;

-- The administrator reprocessing workflow is never authorized by caller
-- supplied actor/video fields. Its redacted request must name the exact
-- immutable parent action receipt, and that parent must still be eligible in
-- this same insert transaction.
CREATE TRIGGER legacy_protected_billing_auth_workflow_parent_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_receipts_v1
WHEN NEW.source_operation_id = 'cap-v1-5a990f470c701cec'
  AND NOT EXISTS (
    SELECT 1
    FROM legacy_protected_billing_auth_receipts_v1 parent
    JOIN legacy_protected_billing_auth_outbox_v1 parent_outbox
      ON parent_outbox.receipt_id = parent.receipt_id
    JOIN legacy_protected_billing_auth_approval_requests_v1 parent_approval
      ON parent_approval.receipt_id = parent.receipt_id
    JOIN legacy_protected_billing_auth_live_authority_v1 parent_live
      ON parent_live.receipt_id = parent.receipt_id
    WHERE parent.receipt_id = json_extract(
        NEW.redacted_request_json,'$.payload._frameParentReceiptId'
      )
      AND parent.request_digest = json_extract(
        NEW.redacted_request_json,'$.payload._frameParentRequestDigest'
      )
      AND parent.source_operation_id = 'cap-v1-14ea978608dcf07e'
      AND parent.operation_kind = 'server_action'
      AND parent.method = 'ACTION'
      AND parent.auth_class = 'admin_session'
      AND parent.authority_class = 'messenger_admin_video'
      AND parent.provider_kind = 'media_reprocess_workflow_dispatch'
      AND parent.human_approval_required = 1
      AND parent.actor_id = NEW.actor_id
      AND parent.target_id = NEW.target_id
      AND parent.credential_kind = 'session_token'
      AND parent.credential_kind = NEW.credential_kind
      AND parent.credential_subject_id = NEW.credential_subject_id
      AND parent.credential_key_version = NEW.credential_key_version
      AND parent.credential_digest = NEW.credential_digest
      AND parent_live.authority_expires_at_ms > NEW.created_at_ms
      AND parent.state NOT IN ('rejected','dead_letter')
      AND parent_outbox.provider_kind = parent.provider_kind
      AND parent_approval.request_digest = parent.request_digest
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_workflow_parent_invalid_v1');
END;

CREATE TABLE legacy_protected_billing_auth_approval_requests_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_billing_auth_receipts_v1(receipt_id) ON DELETE RESTRICT,
  approval_scope TEXT NOT NULL CHECK (approval_scope = 'billing_admin.v1'),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','approved','rejected')),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  resolved_at_ms INTEGER CHECK (
    resolved_at_ms IS NULL OR resolved_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'pending' AND resolved_at_ms IS NULL)
    OR (state IN ('approved','rejected') AND resolved_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_protected_billing_auth_outbox_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_billing_auth_receipts_v1(receipt_id) ON DELETE RESTRICT,
  provider_kind TEXT NOT NULL CHECK (length(provider_kind) BETWEEN 1 AND 96),
  payload_json TEXT NOT NULL CHECK (
    json_valid(payload_json) AND json_type(payload_json) = 'object'
    AND length(payload_json) <= 1048576
  ),
  payload_digest TEXT NOT NULL CHECK (
    length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN (
    'blocked_human_approval','pending_provider_evidence','verified','dead_letter'
  )),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (
    attempt_count BETWEEN 0 AND 1000000
  ),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state IN ('blocked_human_approval','pending_provider_evidence')
      AND completed_at_ms IS NULL)
    OR (state IN ('verified','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_protected_billing_auth_outbox_pending_v1
  ON legacy_protected_billing_auth_outbox_v1(
    state,provider_kind,created_at_ms,receipt_id
  );

-- A webhook provider may generate a new valid signature for every delivery of
-- the same event. Keep those verified delivery credentials as immutable audit
-- rows without placing them in the receipt uniqueness or canonical request
-- digest.
CREATE TABLE legacy_protected_billing_auth_delivery_audit_v1 (
  receipt_id TEXT NOT NULL
    REFERENCES legacy_protected_billing_auth_receipts_v1(receipt_id) ON DELETE RESTRICT,
  transport_credential_digest TEXT NOT NULL CHECK (
    length(transport_credential_digest) = 64
    AND transport_credential_digest NOT GLOB '*[^0-9a-f]*'
  ),
  transport_body_digest TEXT NOT NULL CHECK (
    length(transport_body_digest) = 64
    AND transport_body_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  observed_at_ms INTEGER NOT NULL CHECK (
    observed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY(receipt_id,transport_credential_digest)
);

CREATE TRIGGER legacy_protected_billing_auth_delivery_audit_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_delivery_audit_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_billing_auth_receipts_v1 receipt
  WHERE receipt.receipt_id = NEW.receipt_id
    AND receipt.source_operation_id = 'cap-v1-1e5f228815a2a8b7'
    AND receipt.auth_class = 'signed_webhook'
    AND receipt.authority_class = 'signed_stripe_webhook'
    AND receipt.request_digest = NEW.request_digest
    AND json_extract(
      receipt.redacted_request_json,'$.transport_body_digest'
    ) = NEW.transport_body_digest
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_delivery_audit_invalid_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_delivery_audit_immutable_v1
BEFORE UPDATE ON legacy_protected_billing_auth_delivery_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_delivery_audit_immutable_v1');
END;
CREATE TRIGGER legacy_protected_billing_auth_delivery_audit_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_delivery_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_delivery_audit_immutable_v1');
END;

-- These evidence tables are intentionally not written by the request-path
-- runtime. Separate identities must admit each row.
CREATE TABLE legacy_protected_billing_auth_human_evidence_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_billing_auth_receipts_v1(receipt_id) ON DELETE RESTRICT,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  decision TEXT NOT NULL CHECK (decision IN ('approved','rejected')),
  approver_subject_digest TEXT NOT NULL CHECK (
    length(approver_subject_digest) = 64
    AND approver_subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  approval_evidence_digest TEXT NOT NULL CHECK (
    length(approval_evidence_digest) = 64
    AND approval_evidence_digest NOT GLOB '*[^0-9a-f]*'
  ),
  change_ticket TEXT NOT NULL CHECK (length(change_ticket) BETWEEN 1 AND 255),
  verifier_class TEXT NOT NULL CHECK (verifier_class = 'independent_human_approver'),
  verified_at_ms INTEGER NOT NULL CHECK (
    verified_at_ms BETWEEN 0 AND 9007199254740991
  )
);

CREATE TABLE legacy_protected_billing_auth_provider_evidence_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_billing_auth_receipts_v1(receipt_id) ON DELETE RESTRICT,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  provider_evidence_digest TEXT NOT NULL CHECK (
    length(provider_evidence_digest) = 64
    AND provider_evidence_digest NOT GLOB '*[^0-9a-f]*'
  ),
  -- All provider responses use the sealed HTTP carrier. Checkout/portal URLs,
  -- auth cookies, provider tokens, and signed capability URLs must never be
  -- stored in D1 as JSON, even for non-NextAuth operations.
  sealed_response_ref TEXT NOT NULL CHECK (
    length(sealed_response_ref) = 82
    AND substr(sealed_response_ref,1,18) = 'frame-pba-http-v1:'
    AND substr(sealed_response_ref,19) NOT GLOB '*[^0-9a-f]*'
  ),
  sealed_response_digest TEXT NOT NULL CHECK (
    length(sealed_response_digest) = 64
    AND sealed_response_digest NOT GLOB '*[^0-9a-f]*'
  ),
  verifier_class TEXT NOT NULL CHECK (verifier_class = 'independent_provider_executor'),
  verified_at_ms INTEGER NOT NULL CHECK (
    verified_at_ms BETWEEN 0 AND 9007199254740991
  )
);

CREATE TRIGGER legacy_protected_billing_auth_approval_request_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_approval_requests_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_billing_auth_receipts_v1 receipt
  WHERE receipt.receipt_id = NEW.receipt_id
    AND receipt.human_approval_required = 1
    AND receipt.request_digest = NEW.request_digest
    AND receipt.state = 'awaiting_human_approval'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_approval_request_invalid_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_human_evidence_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_human_evidence_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM legacy_protected_billing_auth_receipts_v1 receipt
  JOIN legacy_protected_billing_auth_approval_requests_v1 approval
    ON approval.receipt_id = receipt.receipt_id
  WHERE receipt.receipt_id = NEW.receipt_id
    AND receipt.human_approval_required = 1
    AND receipt.request_digest = NEW.request_digest
    AND approval.request_digest = NEW.request_digest
    AND approval.state = 'pending'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_human_evidence_invalid_v1');
END;

-- Authority is a condition of provider execution, not merely request
-- admission. Revocation, expiry, ownership transfer, target deletion, or an
-- ineligible workflow parent after staging must prevent evidence from being
-- admitted and therefore prevent projection of a protected success.
CREATE TRIGGER legacy_protected_billing_auth_execution_authority_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_provider_evidence_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM legacy_protected_billing_auth_receipts_v1 receipt
  JOIN legacy_protected_billing_auth_live_authority_v1 live
    ON live.receipt_id = receipt.receipt_id
  WHERE receipt.receipt_id = NEW.receipt_id
    AND receipt.request_digest = NEW.request_digest
    AND NEW.verified_at_ms >= receipt.created_at_ms
    AND live.authority_expires_at_ms > NEW.verified_at_ms
    AND (
      receipt.source_operation_id <> 'cap-v1-5a990f470c701cec'
      OR EXISTS (
        SELECT 1
        FROM legacy_protected_billing_auth_receipts_v1 parent
        JOIN legacy_protected_billing_auth_live_authority_v1 parent_live
          ON parent_live.receipt_id = parent.receipt_id
        JOIN legacy_protected_billing_auth_approval_requests_v1 parent_approval
          ON parent_approval.receipt_id = parent.receipt_id
        JOIN legacy_protected_billing_auth_human_evidence_v1 parent_human
          ON parent_human.receipt_id = parent.receipt_id
        WHERE parent.receipt_id = json_extract(
            receipt.redacted_request_json,'$.payload._frameParentReceiptId'
          )
          AND parent.request_digest = json_extract(
            receipt.redacted_request_json,'$.payload._frameParentRequestDigest'
          )
          AND parent.source_operation_id = 'cap-v1-14ea978608dcf07e'
          AND parent.operation_kind = 'server_action'
          AND parent.method = 'ACTION'
          AND parent.auth_class = 'admin_session'
          AND parent.authority_class = 'messenger_admin_video'
          AND parent.provider_kind = 'media_reprocess_workflow_dispatch'
          AND parent.human_approval_required = 1
          AND parent.actor_id = receipt.actor_id
          AND parent.target_id = receipt.target_id
          AND parent.credential_kind = receipt.credential_kind
          AND parent.credential_subject_id = receipt.credential_subject_id
          AND parent.credential_key_version = receipt.credential_key_version
          AND parent.credential_digest = receipt.credential_digest
          AND parent.state NOT IN ('rejected','dead_letter')
          AND parent_approval.request_digest = parent.request_digest
          AND parent_approval.state = 'approved'
          AND parent_human.request_digest = parent.request_digest
          AND parent_human.decision = 'approved'
          AND parent_live.authority_expires_at_ms > NEW.verified_at_ms
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_execution_authority_stale_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_provider_evidence_gate_v1
BEFORE INSERT ON legacy_protected_billing_auth_provider_evidence_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM legacy_protected_billing_auth_receipts_v1 receipt
  JOIN legacy_protected_billing_auth_outbox_v1 outbox
    ON outbox.receipt_id = receipt.receipt_id
  WHERE receipt.receipt_id = NEW.receipt_id
    AND receipt.request_digest = NEW.request_digest
    AND outbox.state = 'pending_provider_evidence'
    AND (
      receipt.human_approval_required = 0
      OR EXISTS (
        SELECT 1 FROM legacy_protected_billing_auth_human_evidence_v1 human
        WHERE human.receipt_id = receipt.receipt_id
          AND human.request_digest = receipt.request_digest
          AND human.decision = 'approved'
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_provider_evidence_invalid_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_verified_gate_v1
BEFORE UPDATE OF state ON legacy_protected_billing_auth_receipts_v1
WHEN NEW.state = 'verified' AND NOT (
  EXISTS (
    SELECT 1 FROM legacy_protected_billing_auth_provider_evidence_v1 provider
    WHERE provider.receipt_id = OLD.receipt_id
      AND provider.request_digest = OLD.request_digest
  )
  AND (
    OLD.human_approval_required = 0
    OR EXISTS (
      SELECT 1 FROM legacy_protected_billing_auth_human_evidence_v1 human
      WHERE human.receipt_id = OLD.receipt_id
        AND human.request_digest = OLD.request_digest
        AND human.decision = 'approved'
    )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_evidence_required_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_rejected_gate_v1
BEFORE UPDATE OF state ON legacy_protected_billing_auth_receipts_v1
WHEN NEW.state = 'rejected' AND NOT EXISTS (
  SELECT 1 FROM legacy_protected_billing_auth_human_evidence_v1 human
  WHERE human.receipt_id = OLD.receipt_id
    AND human.request_digest = OLD.request_digest
    AND human.decision = 'rejected'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_rejection_evidence_required_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_approval_transition_v1
BEFORE UPDATE ON legacy_protected_billing_auth_approval_requests_v1
WHEN NOT (
  OLD.receipt_id = NEW.receipt_id
  AND OLD.approval_scope = NEW.approval_scope
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'pending'
  AND NEW.state IN ('approved','rejected')
  AND NEW.resolved_at_ms IS NOT NULL
  AND EXISTS (
    SELECT 1 FROM legacy_protected_billing_auth_human_evidence_v1 human
    WHERE human.receipt_id = OLD.receipt_id
      AND human.request_digest = OLD.request_digest
      AND human.decision = NEW.state
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_approval_immutable_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_outbox_transition_v1
BEFORE UPDATE ON legacy_protected_billing_auth_outbox_v1
WHEN NOT (
  OLD.receipt_id = NEW.receipt_id
  AND OLD.provider_kind = NEW.provider_kind
  AND OLD.payload_json = NEW.payload_json
  AND OLD.payload_digest = NEW.payload_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND (
    (
      OLD.state = 'blocked_human_approval'
      AND NEW.state = 'pending_provider_evidence'
      AND NEW.attempt_count = OLD.attempt_count
      AND NEW.completed_at_ms IS NULL
      AND EXISTS (
        SELECT 1 FROM legacy_protected_billing_auth_human_evidence_v1 human
        WHERE human.receipt_id = OLD.receipt_id AND human.decision = 'approved'
      )
    )
    OR (
      OLD.state = 'pending_provider_evidence'
      AND NEW.state IN ('verified','dead_letter')
      AND NEW.attempt_count = OLD.attempt_count + 1
      AND NEW.completed_at_ms IS NOT NULL
      AND (
        NEW.state = 'dead_letter'
        OR EXISTS (
          SELECT 1 FROM legacy_protected_billing_auth_provider_evidence_v1 provider
          WHERE provider.receipt_id = OLD.receipt_id
        )
      )
    )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_outbox_immutable_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_receipt_immutable_v1
BEFORE UPDATE ON legacy_protected_billing_auth_receipts_v1
WHEN NOT (
  OLD.receipt_id = NEW.receipt_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.method = NEW.method
  AND OLD.surface_path = NEW.surface_path
  AND OLD.auth_class = NEW.auth_class
  AND OLD.authority_class = NEW.authority_class
  AND OLD.provider_kind = NEW.provider_kind
  AND OLD.human_approval_required = NEW.human_approval_required
  AND OLD.provider_execution_required = NEW.provider_execution_required
  AND OLD.principal_digest = NEW.principal_digest
  AND OLD.actor_id IS NEW.actor_id
  AND OLD.credential_kind = NEW.credential_kind
  AND OLD.credential_subject_id IS NEW.credential_subject_id
  AND OLD.credential_key_version IS NEW.credential_key_version
  AND OLD.credential_digest IS NEW.credential_digest
  AND OLD.sealed_request_ref IS NEW.sealed_request_ref
  AND OLD.sealed_request_digest IS NEW.sealed_request_digest
  AND OLD.target_id IS NEW.target_id
  AND OLD.replay_key_digest = NEW.replay_key_digest
  AND OLD.replay_origin = NEW.replay_origin
  AND OLD.idempotency_mode = NEW.idempotency_mode
  AND OLD.request_digest = NEW.request_digest
  AND OLD.redacted_request_json = NEW.redacted_request_json
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state IN ('awaiting_human_approval','awaiting_provider_evidence')
  AND NEW.state IN ('verified','rejected','dead_letter')
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_protected_billing_auth_receipt_immutable_v1');
END;

CREATE TRIGGER legacy_protected_billing_auth_receipt_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_protected_billing_auth_outbox_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_protected_billing_auth_approval_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_approval_requests_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_approval_immutable_v1'); END;
CREATE TRIGGER legacy_protected_billing_auth_human_evidence_immutable_v1
BEFORE UPDATE ON legacy_protected_billing_auth_human_evidence_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_human_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_protected_billing_auth_human_evidence_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_human_evidence_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_human_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_protected_billing_auth_provider_evidence_immutable_v1
BEFORE UPDATE ON legacy_protected_billing_auth_provider_evidence_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_provider_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_protected_billing_auth_provider_evidence_no_delete_v1
BEFORE DELETE ON legacy_protected_billing_auth_provider_evidence_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_billing_auth_provider_evidence_immutable_v1'); END;
