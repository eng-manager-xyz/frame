PRAGMA foreign_keys = ON;

-- Contract phase for migration 0028. This file is excluded from normal
-- expand-only releases and may be applied only by the protected contract gate
-- after exact-source N/N-1 compatibility observation.
CREATE TABLE r2_multipart_claim_contract_assertions_v1 (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  assertion TEXT NOT NULL CHECK (
    assertion = 'legacy_provider_mutations_drained'
  ),
  asserted_at_ms INTEGER NOT NULL CHECK (
    asserted_at_ms BETWEEN 1 AND 9007199254740991
  )
) WITHOUT ROWID;

CREATE TRIGGER r2_multipart_claim_contract_assertions_v1_guard
BEFORE INSERT ON r2_multipart_claim_contract_assertions_v1
WHEN NOT EXISTS (
  SELECT 1 FROM r2_multipart_claim_rollout_v1
  WHERE singleton = 1 AND phase = 'fenced' AND updated_at_ms = 0
) OR EXISTS (
  SELECT 1 FROM r2_multipart_sessions_v1
  WHERE state IN ('open', 'completing')
) OR EXISTS (
  SELECT 1 FROM r2_multipart_creation_claims_v1
  WHERE state IN ('reserved', 'provider_bound')
) OR EXISTS (
  SELECT 1 FROM r2_multipart_completion_claims_v1
  WHERE state = 'active'
) OR EXISTS (
  SELECT 1 FROM r2_multipart_part_claims_v1 claim
  LEFT JOIN r2_multipart_parts_v1 part
    ON part.upload_id = claim.upload_id
   AND part.part_number = claim.part_number
   AND part.part_claim_token = claim.claim_token
  WHERE part.upload_id IS NULL
) OR EXISTS (
  SELECT 1 FROM r2_multipart_completion_reconciliation_v1
  WHERE state = 'pending'
) OR EXISTS (
  SELECT 1 FROM r2_multipart_abort_reconciliation_v1
  WHERE state = 'pending'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_claim_contract_not_drained_v1');
END;

CREATE TRIGGER r2_multipart_claim_contract_assertions_v1_immutable
BEFORE UPDATE ON r2_multipart_claim_contract_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_claim_contract_assertion_v1');
END;

CREATE TRIGGER r2_multipart_claim_contract_assertions_v1_no_delete
BEFORE DELETE ON r2_multipart_claim_contract_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_claim_contract_assertion_v1');
END;

INSERT INTO r2_multipart_claim_contract_assertions_v1(
  singleton, assertion, asserted_at_ms
) VALUES (1, 'legacy_provider_mutations_drained', 1);

CREATE TRIGGER r2_multipart_sessions_v1_creation_authority
BEFORE INSERT ON r2_multipart_sessions_v1
WHEN NEW.state != 'open'
  OR NEW.completed_at_ms IS NOT NULL
  OR NOT EXISTS (
    SELECT 1
    FROM r2_multipart_creation_claims_v1 claim
    WHERE claim.upload_id = NEW.upload_id
      AND claim.object_key = NEW.object_key
      AND claim.provider_upload_id = NEW.provider_upload_id
      AND claim.state = 'provider_bound'
      AND claim.expected_bytes = NEW.expected_bytes
      AND claim.checksum_sha256 = NEW.checksum_sha256
      AND claim.content_type = NEW.content_type
      AND claim.correlation_id = NEW.correlation_id
      AND claim.created_at_ms = NEW.created_at_ms
      AND claim.expires_at_ms = NEW.expires_at_ms
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_session_creation_authority_v1');
END;

CREATE TRIGGER r2_multipart_abort_reconciliation_v1_completion_exclusion
BEFORE INSERT ON r2_multipart_abort_reconciliation_v1
WHEN EXISTS (
  SELECT 1
  FROM r2_multipart_sessions_v1 session
  LEFT JOIN r2_multipart_completion_claims_v1 claim USING(upload_id)
  WHERE session.upload_id = NEW.upload_id
    AND (session.state = 'completing' OR claim.state = 'active')
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_completion_abort_exclusion_v1');
END;

CREATE TRIGGER r2_multipart_parts_v1_claim_authority
BEFORE INSERT ON r2_multipart_parts_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM r2_multipart_part_claims_v1 claim
  WHERE claim.upload_id = NEW.upload_id
    AND claim.part_number = NEW.part_number
    AND claim.bytes = NEW.bytes
    AND claim.checksum_sha256 = NEW.checksum_sha256
    AND claim.claim_token = NEW.part_claim_token
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_part_claim_v1');
END;

CREATE TRIGGER r2_multipart_parts_v1_claim_immutable
BEFORE UPDATE OF part_claim_token ON r2_multipart_parts_v1
WHEN NEW.part_claim_token IS NOT OLD.part_claim_token
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_part_claim_v1');
END;

-- Completion and abort are mutually exclusive provider mutations. Completion
-- must first move the session to `completing` while no abort reconciliation is
-- pending; once that commitment exists, an abort can only preserve it.
CREATE TRIGGER r2_multipart_completions_v1_session_authority
BEFORE INSERT ON r2_multipart_completions_v1
WHEN NEW.completion_claim_token IS NULL
OR NOT EXISTS (
  SELECT 1
  FROM r2_multipart_sessions_v1 session
  JOIN r2_multipart_completion_claims_v1 claim USING(upload_id)
  WHERE session.upload_id = NEW.upload_id
    AND session.state = 'completing'
    AND claim.state = 'active'
    AND claim.request_parts_sha256 = NEW.request_parts_sha256
    AND claim.claim_token = NEW.completion_claim_token
)
OR EXISTS (
  SELECT 1
  FROM r2_multipart_abort_reconciliation_v1 reconciliation
  WHERE reconciliation.upload_id = NEW.upload_id
    AND reconciliation.state = 'pending'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_completion_authority_v1');
END;

CREATE TRIGGER r2_multipart_sessions_v1_state_transition
BEFORE UPDATE OF state, completed_at_ms ON r2_multipart_sessions_v1
WHEN NOT (
  (OLD.state = 'open' AND NEW.state IN ('open', 'aborted', 'expired'))
  OR (OLD.state = 'open' AND NEW.state = 'completing'
    AND EXISTS (
      SELECT 1 FROM r2_multipart_completion_claims_v1 claim
      WHERE claim.upload_id = NEW.upload_id
        AND claim.state = 'active'
    ))
  OR (OLD.state = 'completing' AND NEW.state = 'completing')
  OR (OLD.state = 'completing' AND NEW.state = 'complete'
    AND EXISTS (
      SELECT 1
      FROM r2_multipart_completion_claims_v1 claim
      JOIN r2_multipart_completions_v1 completion USING(upload_id)
      WHERE claim.upload_id = NEW.upload_id
        AND claim.state = 'active'
        AND completion.request_parts_sha256 = claim.request_parts_sha256
        AND completion.completion_claim_token = claim.claim_token
        AND completion.bytes = NEW.expected_bytes
        AND completion.checksum_sha256 = NEW.checksum_sha256
        AND completion.content_type = NEW.content_type
        AND completion.correlation_id = NEW.correlation_id
        AND completion.completed_at_ms = NEW.completed_at_ms
    ))
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_session_transition_v1');
END;

-- This is the sole transition that releases new Worker provider mutations.
-- It occurs only after every enforcement trigger and immutable drain proof is
-- installed, so an old rollback bundle can never remain eligible afterward.
UPDATE r2_multipart_claim_rollout_v1
SET phase = 'enabled', updated_at_ms = 1
WHERE singleton = 1 AND phase = 'fenced' AND updated_at_ms = 0;
