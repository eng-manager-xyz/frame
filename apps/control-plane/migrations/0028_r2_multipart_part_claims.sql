PRAGMA foreign_keys = ON;

-- Claim-aware multipart writes cannot overlap an N-1 Worker: provider part,
-- completion, and abort operations occur before that Worker's nullable-token
-- D1 writes. Expansion therefore defaults the new runtime to a hard fence.
-- The protected contract migration enables mutations only after exact active
-- and rollback bundles are claim-aware and every legacy session is drained.
CREATE TABLE r2_multipart_claim_rollout_v1 (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  phase TEXT NOT NULL CHECK (phase IN ('fenced', 'enabled')),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN 0 AND 9007199254740991
  )
) WITHOUT ROWID;

INSERT INTO r2_multipart_claim_rollout_v1(singleton, phase, updated_at_ms)
VALUES (1, 'fenced', 0);

CREATE TRIGGER r2_multipart_claim_rollout_v1_transition
BEFORE UPDATE ON r2_multipart_claim_rollout_v1
WHEN NEW.singleton != OLD.singleton
  OR NOT (
    OLD.phase = 'fenced' AND NEW.phase = 'enabled'
    AND NEW.updated_at_ms > OLD.updated_at_ms
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_claim_rollout_v1');
END;

CREATE TRIGGER r2_multipart_claim_rollout_v1_no_delete
BEFORE DELETE ON r2_multipart_claim_rollout_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_claim_rollout_v1');
END;

-- R2 does not expose an idempotency key for multipart creation. Claim the
-- exact upload intent in D1 before provider I/O, then persist the returned
-- provider handle before creating the public session row. A concurrent caller
-- can reconcile a provider-bound claim but can never create a second upload.
-- A process death after R2 creates the provider upload but before D1 binds its
-- handle cannot be recovered automatically: the durable `reserved` claim
-- fails closed against duplicate creation, and an operator must reconcile the
-- orphan from provider-side metadata before the upload can proceed.
CREATE TABLE r2_multipart_creation_claims_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL
    REFERENCES video_uploads(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL
    REFERENCES organizations(id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL CHECK (length(object_key) BETWEEN 16 AND 1024),
  expected_bytes INTEGER NOT NULL CHECK (
    expected_bytes BETWEEN 1 AND 9007199254740991
  ),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36),
  part_size INTEGER NOT NULL CHECK (part_size BETWEEN 5242880 AND 104857600),
  part_count INTEGER NOT NULL CHECK (part_count BETWEEN 1 AND 10000),
  expires_at_ms INTEGER NOT NULL CHECK (
    expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  claim_token TEXT NOT NULL UNIQUE CHECK (length(claim_token) = 36),
  state TEXT NOT NULL CHECK (state IN (
    'reserved', 'provider_bound', 'committed'
  )),
  provider_upload_id TEXT CHECK (
    provider_upload_id IS NULL
      OR length(provider_upload_id) BETWEEN 1 AND 1024
  ),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (updated_at_ms >= created_at_ms),
  CHECK (
    (state = 'reserved' AND provider_upload_id IS NULL)
    OR (state IN ('provider_bound', 'committed')
      AND provider_upload_id IS NOT NULL)
  )
) WITHOUT ROWID;

CREATE TRIGGER r2_multipart_creation_claims_v1_contract
BEFORE INSERT ON r2_multipart_creation_claims_v1
WHEN NEW.state != 'reserved'
  OR NEW.provider_upload_id IS NOT NULL
  OR NOT EXISTS (
    SELECT 1
    FROM video_uploads upload
    JOIN r2_multipart_intents_v1 intent ON intent.upload_id = upload.id
    JOIN storage_integrations integration ON integration.id = intent.integration_id
    WHERE upload.id = NEW.upload_id
      AND upload.organization_id = NEW.organization_id
      AND upload.source_object_key = NEW.object_key
      AND upload.expected_bytes = NEW.expected_bytes
      AND upload.content_type = NEW.content_type
      AND upload.transfer_mode = 'brokered'
      AND upload.state IN ('initiated', 'uploading')
      AND intent.checksum_sha256 = NEW.checksum_sha256
      AND intent.part_size = NEW.part_size
      AND intent.part_count = NEW.part_count
      AND intent.expires_at_ms = NEW.expires_at_ms
      AND NEW.created_at_ms < NEW.expires_at_ms
      AND integration.organization_id = upload.organization_id
      AND integration.provider = 'r2'
      AND integration.state = 'active'
      AND json_extract(integration.capabilities_json, '$.multipart') = 1
  )
  OR EXISTS (
    SELECT 1 FROM r2_multipart_sessions_v1 session
    WHERE session.upload_id = NEW.upload_id
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_creation_claim_v1');
END;

CREATE TRIGGER r2_multipart_creation_claims_v1_transition
BEFORE UPDATE ON r2_multipart_creation_claims_v1
WHEN NEW.upload_id != OLD.upload_id
  OR NEW.organization_id != OLD.organization_id
  OR NEW.object_key != OLD.object_key
  OR NEW.expected_bytes != OLD.expected_bytes
  OR NEW.checksum_sha256 != OLD.checksum_sha256
  OR NEW.content_type != OLD.content_type
  OR NEW.correlation_id != OLD.correlation_id
  OR NEW.part_size != OLD.part_size
  OR NEW.part_count != OLD.part_count
  OR NEW.expires_at_ms != OLD.expires_at_ms
  OR NEW.claim_token != OLD.claim_token
  OR NEW.created_at_ms != OLD.created_at_ms
  OR NEW.updated_at_ms < OLD.updated_at_ms
  OR NOT (
    (OLD.state = 'reserved' AND NEW.state = 'provider_bound'
      AND OLD.provider_upload_id IS NULL
      AND NEW.provider_upload_id IS NOT NULL)
    OR (OLD.state = 'provider_bound' AND NEW.state = 'committed'
      AND NEW.provider_upload_id = OLD.provider_upload_id
      AND EXISTS (
        SELECT 1 FROM r2_multipart_sessions_v1 session
        WHERE session.upload_id = NEW.upload_id
          AND session.object_key = NEW.object_key
          AND session.provider_upload_id = NEW.provider_upload_id
          AND session.state = 'open'
          AND session.expected_bytes = NEW.expected_bytes
          AND session.checksum_sha256 = NEW.checksum_sha256
          AND session.content_type = NEW.content_type
          AND session.correlation_id = NEW.correlation_id
          AND session.expires_at_ms = NEW.expires_at_ms
      ))
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_creation_claim_v1');
END;

CREATE TRIGGER r2_multipart_creation_claims_v1_no_delete
BEFORE DELETE ON r2_multipart_creation_claims_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_creation_claim_v1');
END;

ALTER TABLE r2_multipart_completions_v1 ADD COLUMN completion_claim_token TEXT
  CHECK (completion_claim_token IS NULL OR length(completion_claim_token) = 36);

-- Completion owns a leased, request-digest-bound claim. The insert trigger
-- performs the sole open -> completing transition in the same D1 transaction;
-- an expired lease may be taken over only for the identical ordered part set.
CREATE TABLE r2_multipart_completion_claims_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL
    REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE RESTRICT,
  request_parts_sha256 TEXT NOT NULL CHECK (
    length(request_parts_sha256) = 64
      AND request_parts_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  claim_token TEXT NOT NULL UNIQUE CHECK (length(claim_token) = 36),
  state TEXT NOT NULL CHECK (state IN ('active', 'complete')),
  attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 1 AND 65535),
  claimed_at_ms INTEGER NOT NULL CHECK (
    claimed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  lease_expires_at_ms INTEGER NOT NULL CHECK (
    lease_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL
      OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (lease_expires_at_ms > claimed_at_ms),
  CHECK (
    (state = 'active' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  )
) WITHOUT ROWID;

CREATE TRIGGER r2_multipart_completion_claims_v1_contract
BEFORE INSERT ON r2_multipart_completion_claims_v1
WHEN NEW.state != 'active'
  OR NEW.attempt_count != 1
  OR NEW.completed_at_ms IS NOT NULL
  OR NOT EXISTS (
    SELECT 1 FROM r2_multipart_sessions_v1 session
    WHERE session.upload_id = NEW.upload_id
      AND ((session.state = 'open'
          AND session.expires_at_ms > NEW.claimed_at_ms)
        OR session.state = 'completing')
  )
  OR EXISTS (
    SELECT 1 FROM r2_multipart_abort_reconciliation_v1 reconciliation
    WHERE reconciliation.upload_id = NEW.upload_id
      AND reconciliation.state = 'pending'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_completion_claim_v1');
END;

CREATE TRIGGER r2_multipart_completion_claims_v1_linearize
AFTER INSERT ON r2_multipart_completion_claims_v1
BEGIN
  UPDATE r2_multipart_sessions_v1
  SET state = 'completing'
  WHERE upload_id = NEW.upload_id
    AND ((state = 'open' AND expires_at_ms > NEW.claimed_at_ms)
      OR state = 'completing')
    AND NOT EXISTS (
      SELECT 1 FROM r2_multipart_abort_reconciliation_v1 reconciliation
      WHERE reconciliation.upload_id = NEW.upload_id
        AND reconciliation.state = 'pending'
    );
  SELECT CASE WHEN changes() != 1
    THEN RAISE(ABORT, 'frame_r2_multipart_completion_claim_v1') END;
END;

CREATE TRIGGER r2_multipart_completion_claims_v1_transition
BEFORE UPDATE ON r2_multipart_completion_claims_v1
WHEN NEW.upload_id != OLD.upload_id
  OR NEW.request_parts_sha256 != OLD.request_parts_sha256
  OR NOT (
    (OLD.state = 'active' AND NEW.state = 'active'
      AND NEW.claim_token != OLD.claim_token
      AND NEW.attempt_count = OLD.attempt_count + 1
      AND NEW.claimed_at_ms >= OLD.lease_expires_at_ms
      AND NEW.lease_expires_at_ms > NEW.claimed_at_ms
      AND NEW.completed_at_ms IS NULL
      AND EXISTS (
        SELECT 1 FROM r2_multipart_sessions_v1 session
        WHERE session.upload_id = NEW.upload_id
          AND session.state = 'completing'
      )
      AND NOT EXISTS (
        SELECT 1 FROM r2_multipart_abort_reconciliation_v1 reconciliation
        WHERE reconciliation.upload_id = NEW.upload_id
          AND reconciliation.state = 'pending'
      ))
    OR (OLD.state = 'active' AND NEW.state = 'complete'
      AND NEW.claim_token = OLD.claim_token
      AND NEW.attempt_count = OLD.attempt_count
      AND NEW.claimed_at_ms = OLD.claimed_at_ms
      AND NEW.lease_expires_at_ms = OLD.lease_expires_at_ms
      AND NEW.completed_at_ms IS NOT NULL
      AND EXISTS (
        SELECT 1 FROM r2_multipart_sessions_v1 session
        JOIN r2_multipart_completions_v1 completion USING(upload_id)
        WHERE session.upload_id = NEW.upload_id
          AND session.state = 'complete'
          AND completion.request_parts_sha256 = NEW.request_parts_sha256
          AND completion.completion_claim_token = NEW.claim_token
      ))
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_completion_claim_v1');
END;

CREATE TRIGGER r2_multipart_completion_claims_v1_no_delete
BEFORE DELETE ON r2_multipart_completion_claims_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_completion_claim_v1');
END;

-- A provider part can be overwritten, while the D1 receipt is immutable. An
-- exact, leased claim therefore wins before provider I/O so concurrent
-- different bytes can never leave R2 and D1 describing different parts.
CREATE TABLE r2_multipart_part_claims_v1 (
  upload_id TEXT NOT NULL
    REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE CASCADE,
  part_number INTEGER NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  claim_token TEXT NOT NULL CHECK (length(claim_token) = 36),
  claimed_at_ms INTEGER NOT NULL CHECK (
    claimed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  lease_expires_at_ms INTEGER NOT NULL CHECK (
    lease_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  PRIMARY KEY (upload_id, part_number),
  CHECK (lease_expires_at_ms > claimed_at_ms)
) WITHOUT ROWID;

CREATE TRIGGER r2_multipart_part_claims_v1_contract
BEFORE INSERT ON r2_multipart_part_claims_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM r2_multipart_sessions_v1 session
  JOIN r2_multipart_intents_v1 intent USING(upload_id)
  WHERE session.upload_id = NEW.upload_id
    AND session.state = 'open'
    AND NEW.claimed_at_ms < session.expires_at_ms
    AND NEW.lease_expires_at_ms <= session.expires_at_ms
    AND NEW.part_number <= intent.part_count
    AND NEW.bytes = CASE
      WHEN NEW.part_number < intent.part_count THEN intent.part_size
      ELSE session.expected_bytes - ((intent.part_count - 1) * intent.part_size)
    END
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_part_claim_v1');
END;

CREATE TRIGGER r2_multipart_part_claims_v1_update_contract
BEFORE UPDATE ON r2_multipart_part_claims_v1
WHEN NEW.upload_id != OLD.upload_id
  OR NEW.part_number != OLD.part_number
  OR NEW.bytes != OLD.bytes
  OR NEW.checksum_sha256 != OLD.checksum_sha256
  OR NEW.claim_token = OLD.claim_token
  OR NEW.claimed_at_ms < OLD.lease_expires_at_ms
  OR NOT EXISTS (
    SELECT 1
    FROM r2_multipart_sessions_v1 session
    WHERE session.upload_id = NEW.upload_id
      AND session.state = 'open'
      AND NEW.claimed_at_ms < session.expires_at_ms
      AND NEW.lease_expires_at_ms <= session.expires_at_ms
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_part_claim_v1');
END;

ALTER TABLE r2_multipart_parts_v1 ADD COLUMN part_claim_token TEXT
  CHECK (part_claim_token IS NULL OR length(part_claim_token) = 36);

-- Triggers that reject the released pre-claim writer live in protected
-- contract migration 0033. Keeping them out of this expand phase allows the
-- old Worker to finish provider I/O and D1 persistence while the claim-aware
-- Worker is deployed. Contract enforcement waits until both N and the approved
-- N-1 rollback bundle are observed claim-aware.
