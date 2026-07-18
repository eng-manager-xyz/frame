PRAGMA foreign_keys = ON;

-- Expand the source-shaped developer compatibility projection created by 0039.
-- API key material remains encrypted and only its keyed digest is queried.
ALTER TABLE legacy_developer_api_keys_v1 ADD COLUMN last_used_at_ms INTEGER
  CHECK (last_used_at_ms IS NULL OR last_used_at_ms BETWEEN 0 AND 9007199254740991);

ALTER TABLE legacy_developer_videos_v1 ADD COLUMN external_user_id TEXT
  CHECK (external_user_id IS NULL OR length(external_user_id) <= 255);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN name TEXT NOT NULL DEFAULT 'Untitled'
  CHECK (length(name) <= 255);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN duration REAL
  CHECK (duration IS NULL OR duration BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN width REAL
  CHECK (width IS NULL OR width BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN height REAL
  CHECK (height IS NULL OR height BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN fps REAL
  CHECK (fps IS NULL OR fps BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN s3_key TEXT
  CHECK (s3_key IS NULL OR length(s3_key) BETWEEN 1 AND 512);
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN transcription_status TEXT
  CHECK (transcription_status IS NULL OR transcription_status IN (
    'PROCESSING','COMPLETE','ERROR','SKIPPED','NO_AUDIO'
  ));
ALTER TABLE legacy_developer_videos_v1 ADD COLUMN metadata_json TEXT
  CHECK (
    metadata_json IS NULL OR (
      json_valid(metadata_json) AND json_type(metadata_json) = 'object'
      AND length(metadata_json) <= 32768
    )
  );
CREATE INDEX legacy_developer_videos_api_list_v1
  ON legacy_developer_videos_v1(app_id, deleted_at_ms, created_at_ms DESC, id);
CREATE INDEX legacy_developer_videos_external_api_v1
  ON legacy_developer_videos_v1(app_id, external_user_id, deleted_at_ms, created_at_ms DESC);

-- One immutable journal drives optional-key replay and provider continuation.
CREATE TABLE legacy_developer_api_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-5914aa6459d24ff1','cap-v1-5c98b9755e4643ba',
    'cap-v1-0d3940728bc19e0e','cap-v1-b6fe5aec600a2e1a',
    'cap-v1-c904ef9c11983a40','cap-v1-1cbfe3ecac36f198'
  )),
  app_id TEXT NOT NULL REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  target_id TEXT CHECK (target_id IS NULL OR length(target_id) <= 1024),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed','effect_pending','complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  UNIQUE (source_operation_id, app_id, idempotency_key_digest),
  CHECK (
    (state IN ('claimed','effect_pending') AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_developer_api_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  status INTEGER NOT NULL CHECK (status BETWEEN 200 AND 599),
  result_kind TEXT NOT NULL CHECK (result_kind IN (
    'success','upload_initiated','part_presigned','video_created'
  )),
  result_json TEXT NOT NULL CHECK (json_valid(result_json) AND length(result_json) <= 65536),
  result_digest TEXT NOT NULL CHECK (
    length(result_digest) = 64 AND result_digest NOT GLOB '*[^0-9a-f]*'
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_developer_provider_outbox_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  effect_kind TEXT NOT NULL CHECK (effect_kind IN (
    'multipart_create','multipart_complete','multipart_abort'
  )),
  payload_digest TEXT NOT NULL CHECK (
    length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending','complete')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'pending' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_developer_multipart_sessions_v1 (
  provider_upload_id TEXT PRIMARY KEY NOT NULL CHECK (length(provider_upload_id) BETWEEN 1 AND 1024),
  app_id TEXT NOT NULL REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL REFERENCES legacy_developer_videos_v1(id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL CHECK (length(object_key) BETWEEN 1 AND 512),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 1 AND 255),
  state TEXT NOT NULL CHECK (state IN ('open','completing','complete','aborting','aborted')),
  initiated_operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  terminal_operation_id TEXT UNIQUE
    REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  CHECK (
    (state = 'open' AND terminal_operation_id IS NULL AND completed_at_ms IS NULL)
    OR (state IN ('completing','aborting')
      AND terminal_operation_id IS NOT NULL AND completed_at_ms IS NULL)
    OR (state IN ('complete','aborted')
      AND terminal_operation_id IS NOT NULL AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_developer_multipart_video_v1
  ON legacy_developer_multipart_sessions_v1(app_id, video_id, state, created_at_ms DESC);

CREATE TABLE legacy_developer_part_capabilities_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  provider_upload_id TEXT NOT NULL
    REFERENCES legacy_developer_multipart_sessions_v1(provider_upload_id) ON DELETE RESTRICT,
  part_number INTEGER NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > issued_at_ms),
  UNIQUE (provider_upload_id, part_number, operation_id)
);

-- Cap charges video completion and daily storage through one account balance.
-- The trigger turns append-only ledger insertion into the atomic balance CAS.
CREATE TABLE legacy_developer_credit_transactions_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  account_id TEXT NOT NULL
    REFERENCES legacy_developer_credit_accounts_v1(id) ON DELETE RESTRICT,
  transaction_type TEXT NOT NULL CHECK (transaction_type IN ('video_create','storage_daily')),
  amount_microcredits INTEGER NOT NULL CHECK (
    amount_microcredits BETWEEN -9007199254740991 AND 0
  ),
  balance_after_microcredits INTEGER NOT NULL CHECK (
    balance_after_microcredits BETWEEN 0 AND 9007199254740991
  ),
  reference_id TEXT CHECK (reference_id IS NULL OR length(reference_id) <= 255),
  reference_type TEXT NOT NULL CHECK (reference_type IN ('developer_video','manual')),
  metadata_json TEXT NOT NULL CHECK (json_valid(metadata_json) AND length(metadata_json) <= 32768),
  operation_id TEXT UNIQUE
    REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (account_id, transaction_type, reference_id),
  CHECK (
    (transaction_type = 'video_create'
      AND reference_type = 'developer_video' AND operation_id IS NOT NULL)
    OR (transaction_type = 'storage_daily'
      AND reference_type = 'manual' AND operation_id IS NULL)
  )
);

CREATE TRIGGER legacy_developer_credit_transaction_guard_v1
BEFORE INSERT ON legacy_developer_credit_transactions_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_developer_credit_accounts_v1 account
  WHERE account.id = NEW.account_id
    AND NEW.balance_after_microcredits = account.balance_microcredits + NEW.amount_microcredits
    AND NEW.balance_after_microcredits >= 0
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_developer_insufficient_credits_v1');
END;

CREATE TRIGGER legacy_developer_credit_transaction_apply_v1
AFTER INSERT ON legacy_developer_credit_transactions_v1
BEGIN
  UPDATE legacy_developer_credit_accounts_v1
  SET balance_microcredits = NEW.balance_after_microcredits,
      updated_at_ms = NEW.created_at_ms,
      revision = revision + 1,
      last_operation_id = NEW.operation_id
  WHERE id = NEW.account_id;
END;

CREATE TRIGGER legacy_developer_credit_transaction_no_update_v1
BEFORE UPDATE ON legacy_developer_credit_transactions_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_ledger_immutable_v1'); END;
CREATE TRIGGER legacy_developer_credit_transaction_no_delete_v1
BEFORE DELETE ON legacy_developer_credit_transactions_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_ledger_immutable_v1'); END;

CREATE TABLE legacy_developer_daily_storage_snapshots_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  app_id TEXT NOT NULL REFERENCES legacy_developer_apps_v1(id) ON DELETE RESTRICT,
  snapshot_date TEXT NOT NULL CHECK (
    length(snapshot_date) = 10
    AND substr(snapshot_date,5,1) = '-'
    AND substr(snapshot_date,8,1) = '-'
  ),
  total_duration_minutes REAL NOT NULL CHECK (
    total_duration_minutes BETWEEN 0 AND 1.7976931348623157e308
  ),
  video_count INTEGER NOT NULL CHECK (video_count BETWEEN 0 AND 9007199254740991),
  microcredits_charged INTEGER NOT NULL CHECK (
    microcredits_charged BETWEEN 0 AND 9007199254740991
  ),
  processed_at_ms INTEGER NOT NULL CHECK (processed_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (app_id, snapshot_date)
);

CREATE TABLE legacy_developer_cron_runs_v1 (
  snapshot_date TEXT PRIMARY KEY NOT NULL CHECK (length(snapshot_date) = 10),
  apps_processed INTEGER NOT NULL CHECK (apps_processed BETWEEN 0 AND 4294967295),
  completed_at_ms INTEGER NOT NULL CHECK (completed_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_developer_api_audit_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT UNIQUE REFERENCES legacy_developer_api_operations_v1(operation_id) ON DELETE RESTRICT,
  source_operation_id TEXT NOT NULL,
  app_digest TEXT NOT NULL CHECK (length(app_digest) = 64),
  target_digest TEXT NOT NULL CHECK (length(target_digest) = 64),
  request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
  result_digest TEXT NOT NULL CHECK (length(result_digest) = 64),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER legacy_developer_api_operation_guard_v1
BEFORE UPDATE ON legacy_developer_api_operations_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.app_id = NEW.app_id
  AND OLD.target_id IS NEW.target_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.completed_at_ms IS NULL
  AND (
    (OLD.state = 'claimed' AND NEW.state = 'effect_pending' AND NEW.completed_at_ms IS NULL)
    OR (OLD.state IN ('claimed','effect_pending') AND NEW.state = 'complete'
      AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_operation_immutable_v1'); END;

CREATE TRIGGER legacy_developer_api_operation_no_delete_v1
BEFORE DELETE ON legacy_developer_api_operations_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_operation_immutable_v1'); END;

-- Multipart terminal ownership is a D1-enforced compare-and-swap. A second
-- completion/abort operation cannot debit credit or enqueue a provider effect
-- after losing the open-session transition race.
CREATE TRIGGER legacy_developer_multipart_session_transition_v1
BEFORE UPDATE ON legacy_developer_multipart_sessions_v1
WHEN NOT (
  OLD.provider_upload_id = NEW.provider_upload_id
  AND OLD.app_id = NEW.app_id
  AND OLD.video_id = NEW.video_id
  AND OLD.object_key = NEW.object_key
  AND OLD.content_type = NEW.content_type
  AND OLD.initiated_operation_id = NEW.initiated_operation_id
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.updated_at_ms >= OLD.updated_at_ms
  AND NEW.revision = OLD.revision + 1
  AND (
    (OLD.state = 'open' AND NEW.state IN ('completing','aborting')
      AND OLD.terminal_operation_id IS NULL
      AND NEW.terminal_operation_id IS NOT NULL
      AND NEW.completed_at_ms IS NULL
      AND EXISTS (
        SELECT 1
        FROM legacy_developer_api_operations_v1 AS operation
        JOIN legacy_developer_videos_v1 AS video ON video.id = NEW.video_id
        WHERE operation.operation_id = NEW.terminal_operation_id
          AND operation.app_id = NEW.app_id
          AND operation.target_id = video.legacy_video_id
          AND operation.state = 'effect_pending'
          AND (
            (NEW.state = 'completing'
              AND operation.source_operation_id = 'cap-v1-5c98b9755e4643ba')
            OR (NEW.state = 'aborting'
              AND operation.source_operation_id = 'cap-v1-5914aa6459d24ff1')
          )
      ))
    OR (OLD.state = 'completing' AND NEW.state = 'complete'
      AND OLD.terminal_operation_id = NEW.terminal_operation_id
      AND NEW.completed_at_ms = NEW.updated_at_ms)
    OR (OLD.state = 'aborting' AND NEW.state = 'aborted'
      AND OLD.terminal_operation_id = NEW.terminal_operation_id
      AND NEW.completed_at_ms = NEW.updated_at_ms)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_multipart_transition_v1'); END;

CREATE TRIGGER legacy_developer_multipart_session_no_delete_v1
BEFORE DELETE ON legacy_developer_multipart_sessions_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_multipart_immutable_v1'); END;

CREATE TRIGGER legacy_developer_terminal_outbox_claim_v1
BEFORE INSERT ON legacy_developer_provider_outbox_v1
WHEN NEW.effect_kind IN ('multipart_complete','multipart_abort') AND NOT EXISTS (
  SELECT 1 FROM legacy_developer_multipart_sessions_v1 AS session
  WHERE session.terminal_operation_id = NEW.operation_id
    AND (
      (NEW.effect_kind = 'multipart_complete' AND session.state = 'completing')
      OR (NEW.effect_kind = 'multipart_abort' AND session.state = 'aborting')
    )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_multipart_claim_lost_v1'); END;

CREATE TRIGGER legacy_developer_provider_outbox_transition_v1
BEFORE UPDATE ON legacy_developer_provider_outbox_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.effect_kind = NEW.effect_kind
  AND OLD.payload_digest = NEW.payload_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND (
    (OLD.state = 'pending' AND NEW.state = 'pending'
      AND NEW.attempt_count = OLD.attempt_count + 1
      AND NEW.completed_at_ms IS NULL)
    OR (OLD.state = 'pending' AND NEW.state = 'complete'
      AND NEW.attempt_count = OLD.attempt_count
      AND NEW.completed_at_ms IS NOT NULL
      AND (
        (NEW.effect_kind = 'multipart_create' AND EXISTS (
          SELECT 1 FROM legacy_developer_multipart_sessions_v1 AS session
          WHERE session.initiated_operation_id = NEW.operation_id
        ))
        OR (NEW.effect_kind = 'multipart_complete' AND EXISTS (
          SELECT 1 FROM legacy_developer_multipart_sessions_v1 AS session
          WHERE session.terminal_operation_id = NEW.operation_id
            AND session.state = 'complete'
        ))
        OR (NEW.effect_kind = 'multipart_abort' AND EXISTS (
          SELECT 1 FROM legacy_developer_multipart_sessions_v1 AS session
          WHERE session.terminal_operation_id = NEW.operation_id
            AND session.state = 'aborted'
        ))
      ))
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_developer_provider_outbox_no_delete_v1
BEFORE DELETE ON legacy_developer_provider_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_developer_part_capability_no_update_v1
BEFORE UPDATE ON legacy_developer_part_capabilities_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_capability_immutable_v1'); END;
CREATE TRIGGER legacy_developer_part_capability_no_delete_v1
BEFORE DELETE ON legacy_developer_part_capabilities_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_capability_immutable_v1'); END;

CREATE TRIGGER legacy_developer_daily_snapshot_no_update_v1
BEFORE UPDATE ON legacy_developer_daily_storage_snapshots_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_snapshot_immutable_v1'); END;
CREATE TRIGGER legacy_developer_daily_snapshot_no_delete_v1
BEFORE DELETE ON legacy_developer_daily_storage_snapshots_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_snapshot_immutable_v1'); END;

CREATE TRIGGER legacy_developer_cron_run_no_update_v1
BEFORE UPDATE ON legacy_developer_cron_runs_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_cron_run_immutable_v1'); END;
CREATE TRIGGER legacy_developer_cron_run_no_delete_v1
BEFORE DELETE ON legacy_developer_cron_runs_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_cron_run_immutable_v1'); END;

CREATE TRIGGER legacy_developer_api_receipt_no_update_v1
BEFORE UPDATE ON legacy_developer_api_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_developer_api_receipt_no_delete_v1
BEFORE DELETE ON legacy_developer_api_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_developer_api_audit_no_update_v1
BEFORE UPDATE ON legacy_developer_api_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_audit_immutable_v1'); END;
CREATE TRIGGER legacy_developer_api_audit_no_delete_v1
BEFORE DELETE ON legacy_developer_api_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_developer_audit_immutable_v1'); END;
