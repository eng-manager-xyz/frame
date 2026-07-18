PRAGMA foreign_keys = ON;

-- Cap stores a comma-separated email/domain restriction on an organization.
-- Native Frame's verified-domain table has different meaning, so preserve the
-- source value rather than deriving one from DNS verification state.
ALTER TABLE organizations ADD COLUMN legacy_allowed_email_restriction TEXT
  CHECK (
    legacy_allowed_email_restriction IS NULL OR
    length(legacy_allowed_email_restriction) <= 4096
  );

-- Restart-safe authority for transcript mutations and protected translation
-- requests. The browser carrier supplies an idempotency key even though a Next
-- server action has no first-class header; this is the explicit Frame safety
-- envelope around R2 effects.
CREATE TABLE legacy_transcript_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-c8dffb9b102dd4f7',
    'cap-v1-3db394ae13895b46',
    'cap-v1-6f6ece85bd786289'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN ('retry', 'edit', 'translate')),
  actor_scope_digest TEXT NOT NULL CHECK (
    length(actor_scope_digest) = 64 AND actor_scope_digest NOT GLOB '*[^0-9a-f]*'
  ),
  actor_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL CHECK (length(legacy_video_id) BETWEEN 1 AND 1020),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  object_key TEXT NOT NULL CHECK (
    length(object_key) BETWEEN 5 AND 1024
    AND object_key NOT LIKE '/%'
    AND object_key NOT LIKE '%\\%'
    AND object_key NOT LIKE '%../%'
  ),
  target_language TEXT CHECK (
    target_language IS NULL OR target_language IN (
      'en','es','fr','de','pt','it','nl','pl','ro','sk','ru','tr','ja',
      'ko','zh','ar','hi','bn','ta','te','mr','gu','pa','ur','fa','he'
    )
  ),
  entry_id INTEGER CHECK (entry_id IS NULL OR entry_id BETWEEN 0 AND 9007199254740991),
  replacement_text TEXT CHECK (replacement_text IS NULL OR length(replacement_text) <= 262144),
  state TEXT NOT NULL CHECK (state IN (
    'claimed', 'storage_applied', 'provider_pending', 'complete', 'failed'
  )),
  result_json TEXT CHECK (
    result_json IS NULL OR (json_valid(result_json) AND length(result_json) <= 262144)
  ),
  failure_code TEXT CHECK (failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 64),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  UNIQUE (
    source_operation_id, actor_scope_digest, mapped_video_id, idempotency_key_digest
  ),
  CHECK (
    (operation_kind = 'retry' AND actor_id IS NOT NULL AND target_language IS NULL
      AND entry_id IS NULL AND replacement_text IS NULL)
    OR (operation_kind = 'edit' AND actor_id IS NOT NULL AND target_language IS NULL
      AND entry_id IS NOT NULL AND replacement_text IS NOT NULL)
    OR (operation_kind = 'translate' AND target_language IS NOT NULL
      AND entry_id IS NULL AND replacement_text IS NULL)
  ),
  CHECK (
    (state = 'complete' AND result_json IS NOT NULL AND completed_at_ms IS NOT NULL)
    OR (state = 'failed' AND failure_code IS NOT NULL)
    OR state IN ('claimed', 'storage_applied', 'provider_pending')
  )
);
CREATE INDEX legacy_transcript_operations_pending_v1
  ON legacy_transcript_operations_v1(state, updated_at_ms, operation_id)
  WHERE state IN ('claimed', 'storage_applied', 'provider_pending');

CREATE TABLE legacy_transcript_storage_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_transcript_operations_v1(operation_id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL,
  prior_etag TEXT,
  applied_etag TEXT NOT NULL CHECK (length(applied_etag) BETWEEN 1 AND 512),
  content_sha256 TEXT NOT NULL CHECK (
    length(content_sha256) = 64 AND content_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  content_bytes INTEGER NOT NULL CHECK (content_bytes BETWEEN 0 AND 8388608),
  applied_at_ms INTEGER NOT NULL CHECK (applied_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_transcript_translation_outbox_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_transcript_operations_v1(operation_id) ON DELETE RESTRICT,
  model TEXT NOT NULL CHECK (model = 'openai/gpt-oss-120b'),
  source_object_key TEXT NOT NULL,
  target_object_key TEXT NOT NULL,
  target_language TEXT NOT NULL,
  prompt_contract_version TEXT NOT NULL CHECK (
    prompt_contract_version = 'cap.translate-vtt.v1'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'leased', 'succeeded', 'dead_letter')),
  lease_token_digest TEXT CHECK (
    lease_token_digest IS NULL OR (
      length(lease_token_digest) = 64 AND lease_token_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  lease_expires_at_ms INTEGER CHECK (
    lease_expires_at_ms IS NULL OR lease_expires_at_ms BETWEEN 0 AND 9007199254740991
  ),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'leased' AND lease_token_digest IS NOT NULL AND lease_expires_at_ms IS NOT NULL)
    OR (state <> 'leased' AND lease_token_digest IS NULL AND lease_expires_at_ms IS NULL)
  )
);
CREATE INDEX legacy_transcript_translation_pending_v1
  ON legacy_transcript_translation_outbox_v1(state, updated_at_ms, operation_id)
  WHERE state IN ('pending', 'leased');

CREATE TRIGGER legacy_transcript_storage_receipt_immutable_update_v1
BEFORE UPDATE ON legacy_transcript_storage_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_transcript_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_transcript_storage_receipt_immutable_delete_v1
BEFORE DELETE ON legacy_transcript_storage_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_transcript_receipt_immutable_v1');
END;
