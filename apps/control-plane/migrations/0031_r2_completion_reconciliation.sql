PRAGMA foreign_keys = ON;

-- Multipart completion crosses R2 and D1. A completion claim makes the
-- provider mutation restart-safe, while this additive journal makes scheduled
-- reconciliation fair: retryable failures receive a bounded lease/backoff and
-- permanent failures become visible quarantine rows instead of poisoning the
-- oldest-session query forever.
CREATE TABLE r2_multipart_completion_reconciliation_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL
    REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE RESTRICT,
  state TEXT NOT NULL CHECK (state IN ('pending', 'quarantined', 'complete')),
  attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 0 AND 12),
  next_attempt_at_ms INTEGER NOT NULL CHECK (
    next_attempt_at_ms BETWEEN 0 AND 9007199254740991
  ),
  last_failure_class TEXT CHECK (last_failure_class IS NULL OR last_failure_class IN (
    'not_found', 'throttled', 'timeout', 'unavailable', 'unauthorized',
    'precondition_failed', 'invalid_request', 'integrity', 'unsupported_capability',
    'quota_exceeded'
  )),
  started_at_ms INTEGER NOT NULL CHECK (
    started_at_ms BETWEEN 0 AND 9007199254740991
  ),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN 0 AND 9007199254740991
  ),
  terminal_at_ms INTEGER CHECK (
    terminal_at_ms IS NULL
      OR terminal_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (updated_at_ms >= started_at_ms),
  CHECK (
    (state = 'pending' AND terminal_at_ms IS NULL)
    OR (state = 'quarantined' AND attempt_count >= 1
      AND last_failure_class IS NOT NULL AND terminal_at_ms IS NOT NULL)
    OR (state = 'complete' AND terminal_at_ms IS NOT NULL)
  )
) WITHOUT ROWID;

CREATE INDEX r2_multipart_completion_reconciliation_v1_due_idx
  ON r2_multipart_completion_reconciliation_v1(
    state, next_attempt_at_ms, updated_at_ms, upload_id
  );

-- Preserve already-linearized sessions during the expand deployment. An old
-- Worker may have left a completing row before this journal existed, including
-- a row without a 0028 completion claim; it becomes immediately eligible and
-- the current runtime can create the exact claim during replay.
INSERT INTO r2_multipart_completion_reconciliation_v1(
  upload_id,state,attempt_count,next_attempt_at_ms,last_failure_class,
  started_at_ms,updated_at_ms,terminal_at_ms
)
SELECT
  candidate.upload_id,
  CASE
    WHEN candidate.state = 'complete' AND candidate.completion_valid = 1 THEN 'complete'
    WHEN candidate.state = 'complete' THEN 'quarantined'
    ELSE 'pending'
  END,
  CASE WHEN candidate.state = 'complete' AND candidate.completion_valid = 0 THEN 1 ELSE 0 END,
  CASE
    WHEN candidate.state = 'complete' THEN candidate.terminal_clock_ms
    ELSE COALESCE(candidate.lease_expires_at_ms, candidate.created_at_ms)
  END,
  CASE
    WHEN candidate.state = 'complete' AND candidate.completion_valid = 0 THEN 'integrity'
    ELSE NULL
  END,
  candidate.started_at_ms,
  CASE
    WHEN candidate.state = 'complete' THEN candidate.terminal_clock_ms
    ELSE candidate.started_at_ms
  END,
  CASE WHEN candidate.state = 'complete' THEN candidate.terminal_clock_ms ELSE NULL END
FROM (
  SELECT
    session.upload_id,
    session.state,
    session.created_at_ms,
    claim.lease_expires_at_ms,
    COALESCE(claim.claimed_at_ms, session.created_at_ms) AS started_at_ms,
    CASE
      WHEN session.completed_at_ms < COALESCE(claim.claimed_at_ms, session.created_at_ms)
        THEN COALESCE(claim.claimed_at_ms, session.created_at_ms)
      ELSE COALESCE(session.completed_at_ms, session.created_at_ms)
    END AS terminal_clock_ms,
    CASE WHEN completion.upload_id IS NOT NULL
      AND completion.bytes = session.expected_bytes
      AND completion.checksum_sha256 = session.checksum_sha256
      AND completion.content_type = session.content_type
      AND completion.correlation_id = session.correlation_id
      AND completion.completed_at_ms = session.completed_at_ms
      AND (
        (completion.completion_claim_token IS NULL AND (
          claim.upload_id IS NULL
          OR (claim.state = 'active'
            AND claim.request_parts_sha256 = completion.request_parts_sha256)
        ))
        OR (completion.completion_claim_token IS NOT NULL
          AND claim.claim_token = completion.completion_claim_token
          AND claim.request_parts_sha256 = completion.request_parts_sha256
          AND claim.state IN ('active', 'complete'))
      )
      THEN 1 ELSE 0 END AS completion_valid
  FROM r2_multipart_sessions_v1 session
  LEFT JOIN r2_multipart_completion_claims_v1 claim USING(upload_id)
  LEFT JOIN r2_multipart_completions_v1 completion USING(upload_id)
  WHERE session.state IN ('completing', 'complete')
) candidate;

-- New and mixed-version Workers both create the journal as a harmless side
-- effect of the existing completion claim. INSERT OR IGNORE keeps an expanded
-- database compatible with a pre-existing backfill row.
CREATE TRIGGER r2_multipart_completion_reconciliation_v1_claim_insert
AFTER INSERT ON r2_multipart_completion_claims_v1
BEGIN
  INSERT OR IGNORE INTO r2_multipart_completion_reconciliation_v1(
    upload_id,state,attempt_count,next_attempt_at_ms,last_failure_class,
    started_at_ms,updated_at_ms,terminal_at_ms
  ) VALUES (
    NEW.upload_id,'pending',0,NEW.lease_expires_at_ms,NULL,
    NEW.claimed_at_ms,NEW.claimed_at_ms,NULL
  );
END;

-- During the expand observation window an N-1 Worker may still perform its
-- released open -> completing update without creating a 0028 claim. Journal
-- that transition as an additive side effect so a crash after provider I/O is
-- visible to the new scheduler. Once the protected 0033 contract phase is
-- applied, only the claim-linearized path can reach this trigger.
CREATE TRIGGER r2_multipart_completion_reconciliation_v1_session_completing
AFTER UPDATE OF state ON r2_multipart_sessions_v1
WHEN OLD.state != 'completing' AND NEW.state = 'completing'
BEGIN
  INSERT OR IGNORE INTO r2_multipart_completion_reconciliation_v1(
    upload_id,state,attempt_count,next_attempt_at_ms,last_failure_class,
    started_at_ms,updated_at_ms,terminal_at_ms
  ) VALUES (
    NEW.upload_id,
    'pending',
    0,
    COALESCE((
      SELECT claim.lease_expires_at_ms
      FROM r2_multipart_completion_claims_v1 claim
      WHERE claim.upload_id = NEW.upload_id AND claim.state = 'active'
    ), NEW.created_at_ms),
    NULL,
    COALESCE((
      SELECT claim.claimed_at_ms
      FROM r2_multipart_completion_claims_v1 claim
      WHERE claim.upload_id = NEW.upload_id AND claim.state = 'active'
    ), NEW.created_at_ms),
    COALESCE((
      SELECT claim.claimed_at_ms
      FROM r2_multipart_completion_claims_v1 claim
      WHERE claim.upload_id = NEW.upload_id AND claim.state = 'active'
    ), NEW.created_at_ms),
    NULL
  );
END;

-- If an N-1 completion races an active N claim for the identical ordered part
-- digest, promote the nullable legacy receipt into that claim after the
-- session reaches complete. The follow-up claim transition uses the existing
-- 0028 authority trigger and prevents a matching active claim from remaining
-- stranded forever. Normal N writes insert a non-null token directly and do
-- not enter this compatibility trigger.
CREATE TRIGGER r2_multipart_completion_reconciliation_v1_legacy_claim_promoted
AFTER UPDATE OF completion_claim_token ON r2_multipart_completions_v1
WHEN OLD.completion_claim_token IS NULL AND NEW.completion_claim_token IS NOT NULL
BEGIN
  UPDATE r2_multipart_completion_claims_v1
  SET state = 'complete', completed_at_ms = NEW.completed_at_ms
  WHERE upload_id = NEW.upload_id
    AND state = 'active'
    AND claim_token = NEW.completion_claim_token
    AND request_parts_sha256 = NEW.request_parts_sha256
    AND EXISTS (
      SELECT 1 FROM r2_multipart_sessions_v1 session
      WHERE session.upload_id = NEW.upload_id
        AND session.state = 'complete'
        AND session.expected_bytes = NEW.bytes
        AND session.checksum_sha256 = NEW.checksum_sha256
        AND session.content_type = NEW.content_type
        AND session.correlation_id = NEW.correlation_id
        AND session.completed_at_ms = NEW.completed_at_ms
    );
END;

-- An N-1 Worker may also finish its nullable-token completion after 0031. The
-- session transition is terminal only when the immutable completion matches
-- the full session identity. A matching concurrent active claim is promoted;
-- no-claim legacy completion is accepted during expand; every other complete
-- shape becomes visible integrity quarantine instead of a pending orphan.
CREATE TRIGGER r2_multipart_completion_reconciliation_v1_session_complete
AFTER UPDATE OF state, completed_at_ms ON r2_multipart_sessions_v1
WHEN OLD.state != 'complete' AND NEW.state = 'complete'
BEGIN
  INSERT OR IGNORE INTO r2_multipart_completion_reconciliation_v1(
    upload_id,state,attempt_count,next_attempt_at_ms,last_failure_class,
    started_at_ms,updated_at_ms,terminal_at_ms
  ) VALUES (
    NEW.upload_id,'pending',0,NEW.created_at_ms,NULL,
    NEW.created_at_ms,NEW.created_at_ms,NULL
  );

  UPDATE r2_multipart_completions_v1
  SET completion_claim_token = (
    SELECT claim.claim_token
    FROM r2_multipart_completion_claims_v1 claim
    WHERE claim.upload_id = NEW.upload_id
      AND claim.state = 'active'
      AND claim.request_parts_sha256 = r2_multipart_completions_v1.request_parts_sha256
  )
  WHERE upload_id = NEW.upload_id
    AND completion_claim_token IS NULL
    AND bytes = NEW.expected_bytes
    AND checksum_sha256 = NEW.checksum_sha256
    AND content_type = NEW.content_type
    AND correlation_id = NEW.correlation_id
    AND completed_at_ms = NEW.completed_at_ms
    AND EXISTS (
      SELECT 1 FROM r2_multipart_completion_claims_v1 claim
      WHERE claim.upload_id = NEW.upload_id
        AND claim.state = 'active'
        AND claim.request_parts_sha256 = r2_multipart_completions_v1.request_parts_sha256
    );

  UPDATE r2_multipart_completion_reconciliation_v1
  SET state = 'complete',
      updated_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END,
      terminal_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END
  WHERE upload_id = NEW.upload_id
    AND state IN ('pending', 'quarantined')
    AND EXISTS (
      SELECT 1
      FROM r2_multipart_completions_v1 completion
      WHERE completion.upload_id = NEW.upload_id
        AND completion.bytes = NEW.expected_bytes
        AND completion.checksum_sha256 = NEW.checksum_sha256
        AND completion.content_type = NEW.content_type
        AND completion.correlation_id = NEW.correlation_id
        AND completion.completed_at_ms = NEW.completed_at_ms
        AND (
          (completion.completion_claim_token IS NULL AND NOT EXISTS (
            SELECT 1 FROM r2_multipart_completion_claims_v1 claim
            WHERE claim.upload_id = NEW.upload_id
          ))
          OR EXISTS (
            SELECT 1 FROM r2_multipart_completion_claims_v1 claim
            WHERE claim.upload_id = NEW.upload_id
              AND claim.state = 'complete'
              AND claim.claim_token = completion.completion_claim_token
              AND claim.request_parts_sha256 = completion.request_parts_sha256
          )
        )
    );

  UPDATE r2_multipart_completion_reconciliation_v1
  SET state = 'quarantined',
      attempt_count = CASE WHEN attempt_count = 0 THEN 1 ELSE attempt_count END,
      next_attempt_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END,
      last_failure_class = 'integrity',
      updated_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END,
      terminal_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END
  WHERE upload_id = NEW.upload_id
    AND state = 'pending'
    AND NOT EXISTS (
      SELECT 1
      FROM r2_multipart_completions_v1 completion
      WHERE completion.upload_id = NEW.upload_id
        AND completion.bytes = NEW.expected_bytes
        AND completion.checksum_sha256 = NEW.checksum_sha256
        AND completion.content_type = NEW.content_type
        AND completion.correlation_id = NEW.correlation_id
        AND completion.completed_at_ms = NEW.completed_at_ms
        AND (
          (completion.completion_claim_token IS NULL AND NOT EXISTS (
            SELECT 1 FROM r2_multipart_completion_claims_v1 claim
            WHERE claim.upload_id = NEW.upload_id
          ))
          OR EXISTS (
            SELECT 1 FROM r2_multipart_completion_claims_v1 claim
            WHERE claim.upload_id = NEW.upload_id
              AND claim.state IN ('active', 'complete')
              AND claim.claim_token = completion.completion_claim_token
              AND claim.request_parts_sha256 = completion.request_parts_sha256
          )
        )
    );
END;

-- A successful old or new Worker terminalizes the additive journal without
-- needing to know that the journal exists. The monotonic max tolerates both a
-- provider timestamp behind the D1 clock and a later scheduler attempt that
-- advanced the journal after the provider object was already complete.
CREATE TRIGGER r2_multipart_completion_reconciliation_v1_claim_complete
AFTER UPDATE OF state, completed_at_ms ON r2_multipart_completion_claims_v1
WHEN OLD.state = 'active' AND NEW.state = 'complete'
BEGIN
  UPDATE r2_multipart_completion_reconciliation_v1
  SET state = 'complete',
      updated_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END,
      terminal_at_ms = CASE
        WHEN NEW.completed_at_ms < updated_at_ms THEN updated_at_ms
        ELSE NEW.completed_at_ms
      END
  WHERE upload_id = NEW.upload_id AND state IN ('pending', 'quarantined');
END;

CREATE TRIGGER r2_multipart_completion_reconciliation_v1_transition
BEFORE UPDATE ON r2_multipart_completion_reconciliation_v1
WHEN NEW.upload_id != OLD.upload_id
  OR NEW.started_at_ms != OLD.started_at_ms
  OR NEW.updated_at_ms < OLD.updated_at_ms
  OR NOT (
    -- Acquire one bounded reconciliation attempt and hold it for the provider
    -- completion lease. A concurrent scheduler cannot acquire the same row.
    (OLD.state = 'pending' AND NEW.state = 'pending'
      AND NEW.attempt_count = OLD.attempt_count + 1
      AND NEW.attempt_count <= 12
      AND NEW.next_attempt_at_ms > NEW.updated_at_ms
      AND NEW.last_failure_class IS NULL
      AND NEW.terminal_at_ms IS NULL)
    OR
    -- Retain a retryable failure at the same attempt with a future backoff.
    (OLD.state = 'pending' AND NEW.state = 'pending'
      AND NEW.attempt_count = OLD.attempt_count
      AND OLD.last_failure_class IS NULL
      AND NEW.last_failure_class IN ('throttled', 'timeout', 'unavailable')
      AND NEW.next_attempt_at_ms > NEW.updated_at_ms
      AND NEW.terminal_at_ms IS NULL)
    OR
    -- Permanent failures quarantine immediately. Retryable failures quarantine
    -- only after the twelfth bounded attempt.
    (OLD.state = 'pending' AND NEW.state = 'quarantined'
      AND ((NEW.attempt_count = OLD.attempt_count AND NEW.attempt_count >= 1)
        OR (OLD.attempt_count = 0 AND NEW.attempt_count = 1
          AND NEW.last_failure_class = 'integrity'))
      AND NEW.last_failure_class IS NOT NULL
      AND (NEW.last_failure_class NOT IN ('throttled', 'timeout', 'unavailable')
        OR NEW.attempt_count = 12)
      AND NEW.next_attempt_at_ms = NEW.updated_at_ms
      AND NEW.terminal_at_ms = NEW.updated_at_ms)
    OR
    -- An authoritative late completion may refine a quarantined row to the
    -- true provider/D1 terminal outcome, but no scheduler can reopen it.
    (OLD.state IN ('pending', 'quarantined') AND NEW.state = 'complete'
      AND NEW.attempt_count = OLD.attempt_count
      AND NEW.next_attempt_at_ms = OLD.next_attempt_at_ms
      AND NEW.last_failure_class IS OLD.last_failure_class
      AND NEW.terminal_at_ms = NEW.updated_at_ms
      AND EXISTS (
        SELECT 1
        FROM r2_multipart_sessions_v1 session
        JOIN r2_multipart_completions_v1 completion USING(upload_id)
        WHERE session.upload_id = NEW.upload_id
          AND session.state = 'complete'
          AND completion.bytes = session.expected_bytes
          AND completion.checksum_sha256 = session.checksum_sha256
          AND completion.content_type = session.content_type
          AND completion.correlation_id = session.correlation_id
          AND completion.completed_at_ms = session.completed_at_ms
          AND (
            (completion.completion_claim_token IS NULL AND NOT EXISTS (
              SELECT 1 FROM r2_multipart_completion_claims_v1 claim
              WHERE claim.upload_id = NEW.upload_id
            ))
            OR EXISTS (
              SELECT 1 FROM r2_multipart_completion_claims_v1 claim
              WHERE claim.upload_id = NEW.upload_id
                AND claim.state = 'complete'
                AND claim.claim_token = completion.completion_claim_token
                AND claim.request_parts_sha256 = completion.request_parts_sha256
            )
          )
      ))
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_completion_reconciliation_v1');
END;

-- Reconcile the same matching N-1/N race when it already existed before this
-- migration. Updating the nullable receipt activates the compatibility trigger
-- above, which completes the exact active claim under its 0028 transition
-- guard. A mismatched claim/receipt remains quarantined and untouched.
UPDATE r2_multipart_completions_v1
SET completion_claim_token = (
  SELECT claim.claim_token
  FROM r2_multipart_completion_claims_v1 claim
  WHERE claim.upload_id = r2_multipart_completions_v1.upload_id
    AND claim.state = 'active'
    AND claim.request_parts_sha256 = r2_multipart_completions_v1.request_parts_sha256
)
WHERE completion_claim_token IS NULL
  AND EXISTS (
    SELECT 1
    FROM r2_multipart_sessions_v1 session
    JOIN r2_multipart_completion_claims_v1 claim USING(upload_id)
    WHERE session.upload_id = r2_multipart_completions_v1.upload_id
      AND session.state = 'complete'
      AND session.expected_bytes = r2_multipart_completions_v1.bytes
      AND session.checksum_sha256 = r2_multipart_completions_v1.checksum_sha256
      AND session.content_type = r2_multipart_completions_v1.content_type
      AND session.correlation_id = r2_multipart_completions_v1.correlation_id
      AND session.completed_at_ms = r2_multipart_completions_v1.completed_at_ms
      AND claim.state = 'active'
      AND claim.request_parts_sha256 = r2_multipart_completions_v1.request_parts_sha256
  );

CREATE TRIGGER r2_multipart_completion_reconciliation_v1_no_delete
BEFORE DELETE ON r2_multipart_completion_reconciliation_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_completion_reconciliation_v1');
END;
