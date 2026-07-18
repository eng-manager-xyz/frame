PRAGMA foreign_keys = ON;

ALTER TABLE command_idempotency ADD COLUMN reservation_id TEXT
  CHECK (reservation_id IS NULL OR length(reservation_id) = 36);

CREATE INDEX media_jobs_native_claim_idx
  ON media_jobs(organization_id, selected_executor, state, lease_expires_at_ms, created_at_ms)
  WHERE selected_executor = 'native_gstreamer';

CREATE TRIGGER media_jobs_progress_monotonic
BEFORE UPDATE OF progress_basis_points ON media_jobs
WHEN NEW.attempt = OLD.attempt
  AND OLD.progress_basis_points IS NOT NULL
  AND NEW.progress_basis_points IS NOT NULL
  AND NEW.progress_basis_points < OLD.progress_basis_points
BEGIN
  SELECT RAISE(ABORT, 'media job progress cannot regress within an attempt');
END;

CREATE TRIGGER media_jobs_terminal_state_immutable
BEFORE UPDATE OF state ON media_jobs
WHEN OLD.state IN ('succeeded', 'failed', 'cancelled')
  AND NEW.state <> OLD.state
BEGIN
  SELECT RAISE(ABORT, 'terminal media job state is immutable');
END;
