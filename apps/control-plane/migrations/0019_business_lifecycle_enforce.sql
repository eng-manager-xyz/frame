PRAGMA foreign_keys = ON;

-- Enforce ordered lifecycles, credit accounting, and the dirty-data cutover
-- view in a final bounded D1 authority phase.

CREATE TRIGGER outbox_ordered_transition_v1
BEFORE UPDATE OF state, event_sequence, event_fingerprint ON outbox_events
WHEN NEW.event_sequence <> OLD.event_sequence + 1
  OR NEW.event_fingerprint IS NULL
  OR NOT (
    (OLD.state = 'pending' AND NEW.state = 'leased')
    OR (OLD.state = 'leased' AND NEW.state IN ('pending','delivered','dead_letter'))
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_business_event_order_conflict_v1');
END;

CREATE TRIGGER imported_videos_ordered_transition_v1
BEFORE UPDATE OF state, event_sequence, event_fingerprint, error_class ON imported_videos
WHEN NEW.event_sequence <> OLD.event_sequence + 1
  OR NEW.event_fingerprint IS NULL
  OR (NEW.state = 'failed') <> (NEW.error_class IS NOT NULL)
  OR (NEW.error_class IS NOT NULL AND (
    length(NEW.error_class) NOT BETWEEN 1 AND 64
    OR NEW.error_class <> lower(NEW.error_class)
    OR NEW.error_class GLOB '*[^a-z0-9_]*'
  ))
  OR NOT (
    (OLD.state = 'queued' AND NEW.state IN ('running','cancelled'))
    OR (OLD.state = 'running' AND NEW.state IN ('complete','failed','cancelled'))
    OR (OLD.state = 'failed' AND NEW.state IN ('running','cancelled'))
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_business_event_order_conflict_v1');
END;

CREATE TRIGGER video_uploads_ordered_transition_v1
BEFORE UPDATE OF state, event_sequence, event_fingerprint ON video_uploads
WHEN NEW.event_sequence <> OLD.event_sequence + 1
  OR NEW.event_fingerprint IS NULL
  OR NEW.received_bytes < OLD.received_bytes
  OR NEW.received_bytes > NEW.expected_bytes
  OR (NEW.state = 'complete') <> (NEW.checksum_sha256 IS NOT NULL)
  OR NOT (
    (OLD.state = 'initiated' AND NEW.state IN ('uploading','failed','aborted'))
    OR (OLD.state = 'uploading' AND NEW.state IN ('finalizing','failed','aborted'))
    OR (OLD.state = 'finalizing' AND NEW.state IN ('complete','failed','aborted'))
    OR (OLD.state = 'failed' AND NEW.state IN ('uploading','aborted'))
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_business_event_order_conflict_v1');
END;

CREATE TRIGGER developer_credit_transactions_accounting_v1
BEFORE INSERT ON developer_credit_transactions
WHEN NEW.ledger_sequence IS NULL
  OR NEW.reference_digest IS NULL
  OR NEW.operation_id IS NULL
  OR NEW.request_fingerprint IS NULL
  OR NOT EXISTS (
    SELECT 1 FROM developer_credit_accounts account
    WHERE account.id = NEW.account_id
      AND NEW.ledger_sequence = account.ledger_sequence + 1
      AND NEW.balance_after_microcredits = account.balance_microcredits + NEW.amount_microcredits
      AND NEW.balance_after_microcredits BETWEEN 0 AND 9007199254740991
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_business_accounting_conflict_v1');
END;

CREATE TRIGGER developer_credit_transactions_apply_v1
AFTER INSERT ON developer_credit_transactions
WHEN NEW.ledger_sequence IS NOT NULL
BEGIN
  UPDATE developer_credit_accounts
  SET balance_microcredits = NEW.balance_after_microcredits,
      ledger_sequence = NEW.ledger_sequence,
      revision = revision + 1,
      updated_at_ms = NEW.created_at_ms,
      last_operation_id = NEW.operation_id
  WHERE id = NEW.account_id;
END;

CREATE TRIGGER usage_ledger_authority_v1
BEFORE INSERT ON usage_ledger
WHEN NEW.operation_id IS NULL
  OR NEW.request_fingerprint IS NULL
  OR NOT (
    NEW.organization_id IS NOT NULL
    OR EXISTS (
      SELECT 1 FROM developer_apps app
      WHERE app.id = NEW.app_id AND app.organization_id IS NOT NULL
    )
  )
  OR (
    NEW.app_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM developer_apps app
      WHERE app.id = NEW.app_id AND app.organization_id = NEW.organization_id
    )
  )
  OR (
    NEW.video_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM videos video
      WHERE video.id = NEW.video_id AND video.organization_id = NEW.organization_id
    )
  )
  OR (
    NEW.media_job_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM media_jobs job
      WHERE job.id = NEW.media_job_id AND job.organization_id = NEW.organization_id
    )
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_business_accounting_conflict_v1');
END;
