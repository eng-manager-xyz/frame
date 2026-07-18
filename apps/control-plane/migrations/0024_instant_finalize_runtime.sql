PRAGMA foreign_keys = ON;

-- `video_uploads.transfer_mode = 'brokered'` remains the released high-level
-- transport contract. This expansion records the stricter multipart geometry
-- without rebuilding that table or exposing an R2 upload handle.
CREATE TABLE r2_multipart_intents_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL REFERENCES video_uploads(id) ON DELETE CASCADE,
  integration_id TEXT NOT NULL REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64 AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  part_size INTEGER NOT NULL CHECK (part_size BETWEEN 5242880 AND 104857600),
  part_count INTEGER NOT NULL CHECK (part_count BETWEEN 1 AND 10000),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 1 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms)
);

CREATE TRIGGER r2_multipart_intents_v1_contract
BEFORE INSERT ON r2_multipart_intents_v1
WHEN NOT EXISTS (
  SELECT 1 FROM video_uploads u WHERE u.id = NEW.upload_id
    AND u.transfer_mode = 'brokered' AND u.state = 'initiated'
    AND NEW.part_count = CAST((u.expected_bytes + NEW.part_size - 1) / NEW.part_size AS INTEGER)
) OR NOT EXISTS (
  SELECT 1 FROM storage_integrations i JOIN video_uploads u ON u.id = NEW.upload_id
  WHERE i.id = NEW.integration_id AND i.organization_id = u.organization_id
    AND i.provider = 'r2' AND i.state = 'active'
    AND json_extract(i.capabilities_json, '$.multipart') = 1
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_intent_v1');
END;

-- Completion and publication cannot be one atomic provider/D1 operation. This
-- immutable receipt is written only after the Worker has streamed and hashed
-- the complete R2 object. It lets the scheduled probe bootstrap distinguish a
-- provider-completed object from an untrusted client completion claim.
CREATE TABLE r2_multipart_verified_objects_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL
    REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE RESTRICT,
  provider_version TEXT NOT NULL CHECK (length(provider_version) BETWEEN 1 AND 256),
  provider_etag TEXT NOT NULL CHECK (length(provider_etag) BETWEEN 1 AND 256),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64 AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  verified_at_ms INTEGER NOT NULL CHECK (verified_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER r2_multipart_verified_objects_v1_contract
BEFORE INSERT ON r2_multipart_verified_objects_v1
WHEN NOT EXISTS (
  SELECT 1 FROM r2_multipart_sessions_v1 s
  WHERE s.upload_id = NEW.upload_id AND s.state = 'completing'
    AND s.expected_bytes = NEW.bytes
    AND s.checksum_sha256 = NEW.checksum_sha256
    AND s.content_type = NEW.content_type
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_verified_object_v1');
END;

CREATE TRIGGER r2_multipart_verified_objects_v1_immutable
BEFORE UPDATE ON r2_multipart_verified_objects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_verified_object_v1');
END;

CREATE TRIGGER r2_multipart_verified_objects_v1_no_delete
BEFORE DELETE ON r2_multipart_verified_objects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_verified_object_v1');
END;

-- Provider abort is a two-system reconciliation, not a best-effort delete.
-- Expiry keeps the session `open` while retryable and reaches `expired` only
-- after abort success or authoritative provider not-found. Authenticated DELETE
-- durably takes over a pending expiry intent, then atomically transitions both
-- the session and tenant upload only after the same provider terminal result.
CREATE TABLE r2_multipart_abort_reconciliation_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL
    REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE RESTRICT,
  intent_kind TEXT NOT NULL DEFAULT 'expiry_cleanup'
    CHECK (intent_kind IN ('expiry_cleanup', 'authenticated_delete')),
  state TEXT NOT NULL CHECK (state IN ('pending', 'confirmed', 'preserved_object')),
  attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 1 AND 65535),
  next_attempt_at_ms INTEGER NOT NULL CHECK (next_attempt_at_ms BETWEEN 0 AND 9007199254740991),
  last_failure_class TEXT CHECK (last_failure_class IS NULL OR last_failure_class IN (
    'not_found', 'throttled', 'timeout', 'unavailable', 'unauthorized',
    'precondition_failed', 'invalid_request', 'integrity', 'unsupported_capability',
    'quota_exceeded'
  )),
  started_at_ms INTEGER NOT NULL CHECK (started_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  terminal_at_ms INTEGER CHECK (terminal_at_ms IS NULL OR terminal_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (updated_at_ms >= started_at_ms),
  CHECK (
    (state = 'pending' AND terminal_at_ms IS NULL)
    OR (state IN ('confirmed', 'preserved_object') AND terminal_at_ms IS NOT NULL)
  )
);
CREATE INDEX r2_multipart_abort_reconciliation_v1_due_idx
  ON r2_multipart_abort_reconciliation_v1(
    intent_kind, state, next_attempt_at_ms, updated_at_ms, upload_id
  );

CREATE TRIGGER r2_multipart_abort_reconciliation_v1_scope
BEFORE INSERT ON r2_multipart_abort_reconciliation_v1
WHEN NEW.state <> 'pending' OR NOT EXISTS (
  SELECT 1 FROM r2_multipart_sessions_v1 s
  WHERE s.upload_id = NEW.upload_id
    AND ((NEW.intent_kind = 'expiry_cleanup' AND s.state = 'open')
      OR (NEW.intent_kind = 'authenticated_delete' AND s.state IN ('open', 'completing')))
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_abort_reconciliation_v1');
END;

CREATE TRIGGER r2_multipart_abort_reconciliation_v1_transition
BEFORE UPDATE ON r2_multipart_abort_reconciliation_v1
WHEN NEW.upload_id <> OLD.upload_id
  OR (NEW.intent_kind <> OLD.intent_kind
    AND NOT (OLD.intent_kind = 'expiry_cleanup'
      AND NEW.intent_kind = 'authenticated_delete' AND OLD.state = 'pending'))
  OR OLD.state <> 'pending'
  OR NEW.attempt_count < OLD.attempt_count
  OR NEW.attempt_count > OLD.attempt_count + 1
  OR (NEW.state = 'pending' AND NOT EXISTS (
    SELECT 1 FROM r2_multipart_sessions_v1 s
    WHERE s.upload_id = NEW.upload_id
      AND ((NEW.intent_kind = 'expiry_cleanup' AND s.state = 'open')
        OR (NEW.intent_kind = 'authenticated_delete' AND s.state IN ('open', 'completing')))
  ))
  OR (NEW.state = 'confirmed' AND NOT EXISTS (
    SELECT 1 FROM r2_multipart_sessions_v1 s
    WHERE s.upload_id = NEW.upload_id
      AND ((NEW.intent_kind = 'expiry_cleanup' AND s.state = 'expired')
        OR (NEW.intent_kind = 'authenticated_delete' AND s.state = 'aborted'))
  ))
  OR (NEW.state = 'preserved_object' AND NOT EXISTS (
    SELECT 1 FROM r2_multipart_sessions_v1 s
    WHERE s.upload_id = NEW.upload_id AND s.state = 'completing'
  ))
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_abort_reconciliation_v1');
END;

CREATE TRIGGER r2_multipart_abort_reconciliation_v1_no_delete
BEFORE DELETE ON r2_multipart_abort_reconciliation_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_abort_reconciliation_v1');
END;

CREATE TABLE r2_multipart_abort_terminal_assertions_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL
    REFERENCES r2_multipart_abort_reconciliation_v1(upload_id) ON DELETE RESTRICT,
  outcome TEXT NOT NULL CHECK (outcome IN ('confirmed', 'preserved_object')),
  asserted_at_ms INTEGER NOT NULL CHECK (asserted_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER r2_multipart_abort_terminal_assertions_v1_contract
BEFORE INSERT ON r2_multipart_abort_terminal_assertions_v1
WHEN NOT EXISTS (
  SELECT 1 FROM r2_multipart_abort_reconciliation_v1 reconciliation
  JOIN r2_multipart_sessions_v1 session USING(upload_id)
  WHERE reconciliation.upload_id = NEW.upload_id
    AND reconciliation.state = NEW.outcome
    AND reconciliation.terminal_at_ms IS NOT NULL
    AND ((NEW.outcome = 'confirmed'
        AND ((reconciliation.intent_kind = 'expiry_cleanup' AND session.state = 'expired')
          OR (reconciliation.intent_kind = 'authenticated_delete' AND session.state = 'aborted'
            AND EXISTS (SELECT 1 FROM video_uploads upload
              WHERE upload.id = session.upload_id AND upload.state = 'aborted'))))
      OR (NEW.outcome = 'preserved_object' AND session.state = 'completing'))
)
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_abort_terminal_v1');
END;

CREATE TRIGGER r2_multipart_abort_terminal_assertions_v1_immutable
BEFORE UPDATE ON r2_multipart_abort_terminal_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_abort_terminal_v1');
END;

CREATE TRIGGER r2_multipart_abort_terminal_assertions_v1_no_delete
BEFORE DELETE ON r2_multipart_abort_terminal_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_r2_multipart_abort_terminal_v1');
END;

-- Ephemeral changed-row assertions make the multipart session, reconciliation,
-- authenticated upload state, and terminal receipt one all-or-nothing batch.
-- Successful callers delete these rows before commit; a zero-row transition
-- violates the equality check and rolls the complete D1 batch back while the
-- durable pending reconciliation remains available for retry.
CREATE TABLE r2_multipart_abort_batch_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  upload_id TEXT NOT NULL REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE RESTRICT,
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'attempt_claim', 'failure_retained',
    'session_transition', 'reconciliation_transition',
    'video_upload_transition', 'terminal_assertion'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count = 1),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

-- The desktop may seal a finalize request before the provider completion is
-- visible. This row is the durable reconciliation fence between that request,
-- R2 multipart completion, and the single playable publication.
CREATE TABLE instant_finalize_requests_v1 (
  session_id TEXT PRIMARY KEY NOT NULL CHECK (length(session_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  upload_id TEXT NOT NULL UNIQUE REFERENCES video_uploads(id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  ordered_parts_sha256 TEXT NOT NULL CHECK (
    length(ordered_parts_sha256) = 64 AND ordered_parts_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  object_version TEXT NOT NULL CHECK (
    length(object_version) = 64 AND object_version NOT GLOB '*[^0-9a-f]*'
  ),
  job_id TEXT NOT NULL UNIQUE CHECK (length(job_id) = 36),
  job_generation INTEGER NOT NULL CHECK (job_generation BETWEEN 1 AND 9007199254740991),
  request_sha256 TEXT NOT NULL UNIQUE CHECK (
    length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'published', 'dead_letter')),
  publication_id TEXT UNIQUE CHECK (publication_id IS NULL OR length(publication_id) = 36),
  playable_object_key TEXT,
  distribution_eligible INTEGER NOT NULL DEFAULT 0 CHECK (distribution_eligible IN (0, 1)),
  reconcile_attempt_count INTEGER NOT NULL DEFAULT 0
    CHECK (reconcile_attempt_count BETWEEN 0 AND 65535),
  next_attempt_at_ms INTEGER NOT NULL CHECK (next_attempt_at_ms BETWEEN 0 AND 9007199254740991),
  last_failure_class TEXT CHECK (last_failure_class IS NULL OR last_failure_class IN (
    'dependency_pending', 'authority_unavailable', 'persistence', 'conflict'
  )),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  published_at_ms INTEGER CHECK (published_at_ms IS NULL OR published_at_ms BETWEEN 0 AND 9007199254740991),
  dead_lettered_at_ms INTEGER CHECK (
    dead_lettered_at_ms IS NULL OR dead_lettered_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (updated_at_ms >= created_at_ms),
  CHECK (
    (state = 'pending' AND publication_id IS NULL AND playable_object_key IS NULL
      AND distribution_eligible = 0 AND published_at_ms IS NULL AND dead_lettered_at_ms IS NULL)
    OR (state = 'published' AND publication_id IS NOT NULL AND playable_object_key IS NOT NULL
      AND distribution_eligible = 1 AND published_at_ms IS NOT NULL AND dead_lettered_at_ms IS NULL)
    OR (state = 'dead_letter' AND publication_id IS NULL AND playable_object_key IS NULL
      AND distribution_eligible = 0 AND published_at_ms IS NULL
      AND dead_lettered_at_ms IS NOT NULL AND last_failure_class IS NOT NULL)
  )
);
CREATE INDEX instant_finalize_requests_v1_pending_idx
  ON instant_finalize_requests_v1(state, next_attempt_at_ms, session_id);

CREATE TRIGGER instant_finalize_requests_v1_scope
BEFORE INSERT ON instant_finalize_requests_v1
WHEN NOT EXISTS (
  SELECT 1 FROM video_uploads u
  WHERE u.id = NEW.upload_id AND u.organization_id = NEW.organization_id
    AND u.video_id = NEW.video_id
) OR NOT EXISTS (
  SELECT 1 FROM videos v
  WHERE v.id = NEW.video_id AND v.organization_id = NEW.organization_id
    AND v.deleted_at_ms IS NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_scope_v1');
END;

CREATE TRIGGER instant_finalize_requests_v1_identity_immutable
BEFORE UPDATE ON instant_finalize_requests_v1
WHEN NEW.session_id <> OLD.session_id
  OR NEW.organization_id <> OLD.organization_id
  OR NEW.upload_id <> OLD.upload_id
  OR NEW.video_id <> OLD.video_id
  OR NEW.ordered_parts_sha256 <> OLD.ordered_parts_sha256
  OR NEW.object_version <> OLD.object_version
  OR NEW.job_id <> OLD.job_id
  OR NEW.job_generation <> OLD.job_generation
  OR NEW.request_sha256 <> OLD.request_sha256
  OR OLD.state IN ('published', 'dead_letter')
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_conflict_v1');
END;

CREATE TRIGGER instant_finalize_requests_v1_no_delete
BEFORE DELETE ON instant_finalize_requests_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_conflict_v1');
END;

-- Retaining the requested generation independently prevents a stale callback
-- or a reused job identifier from publishing a second recording.
CREATE TABLE instant_finalize_jobs_v1 (
  job_id TEXT PRIMARY KEY NOT NULL,
  session_id TEXT NOT NULL UNIQUE REFERENCES instant_finalize_requests_v1(session_id) ON DELETE RESTRICT,
  generation INTEGER NOT NULL CHECK (generation BETWEEN 1 AND 9007199254740991),
  request_sha256 TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('retained', 'published', 'cancelled')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (updated_at_ms >= created_at_ms),
  FOREIGN KEY (job_id) REFERENCES instant_finalize_requests_v1(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (request_sha256) REFERENCES instant_finalize_requests_v1(request_sha256) ON DELETE RESTRICT
);

CREATE TABLE instant_finalize_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  session_id TEXT NOT NULL REFERENCES instant_finalize_requests_v1(session_id) ON DELETE RESTRICT,
  request_sha256 TEXT NOT NULL,
  result_state TEXT NOT NULL CHECK (result_state IN ('pending', 'published', 'dead_letter')),
  publication_id TEXT,
  committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms BETWEEN 0 AND 9007199254740991),
  FOREIGN KEY (request_sha256) REFERENCES instant_finalize_requests_v1(request_sha256) ON DELETE RESTRICT,
  CHECK ((result_state = 'published') = (publication_id IS NOT NULL))
);
CREATE INDEX instant_finalize_operations_v1_session_idx
  ON instant_finalize_operations_v1(session_id, committed_at_ms);

CREATE TRIGGER instant_finalize_jobs_v1_identity_immutable
BEFORE UPDATE ON instant_finalize_jobs_v1
WHEN NEW.job_id <> OLD.job_id OR NEW.session_id <> OLD.session_id
  OR NEW.generation <> OLD.generation OR NEW.request_sha256 <> OLD.request_sha256
  OR NEW.created_at_ms <> OLD.created_at_ms
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_job_v1');
END;

CREATE TRIGGER instant_finalize_jobs_v1_no_delete
BEFORE DELETE ON instant_finalize_jobs_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_job_v1');
END;

CREATE TRIGGER instant_finalize_operations_v1_identity_immutable
BEFORE UPDATE ON instant_finalize_operations_v1
WHEN NEW.operation_id <> OLD.operation_id OR NEW.session_id <> OLD.session_id
  OR NEW.request_sha256 <> OLD.request_sha256 OR NEW.committed_at_ms <> OLD.committed_at_ms
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_operation_v1');
END;

CREATE TRIGGER instant_finalize_operations_v1_no_delete
BEFORE DELETE ON instant_finalize_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_operation_v1');
END;

-- The HTTP retry key, semantic request, retained job, and operation are one
-- reservation. The assertion is the final statement in that fenced batch and
-- turns every zero-row conflict into a transactional abort.
CREATE TABLE instant_finalize_http_idempotency_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  idempotency_key TEXT NOT NULL CHECK (
    length(idempotency_key) BETWEEN 8 AND 128
      AND idempotency_key NOT GLOB '*[^A-Za-z0-9_.:-]*'
  ),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES instant_finalize_operations_v1(operation_id) ON DELETE RESTRICT,
  session_id TEXT NOT NULL REFERENCES instant_finalize_requests_v1(session_id) ON DELETE RESTRICT,
  request_sha256 TEXT NOT NULL
    REFERENCES instant_finalize_requests_v1(request_sha256) ON DELETE RESTRICT,
  job_id TEXT NOT NULL REFERENCES instant_finalize_jobs_v1(job_id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, idempotency_key)
);

CREATE TABLE instant_finalize_reservation_assertions_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES instant_finalize_operations_v1(operation_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  asserted_at_ms INTEGER NOT NULL CHECK (asserted_at_ms BETWEEN 0 AND 9007199254740991),
  FOREIGN KEY (organization_id, idempotency_key)
    REFERENCES instant_finalize_http_idempotency_v1(organization_id, idempotency_key)
      ON DELETE RESTRICT
);

CREATE TRIGGER instant_finalize_http_idempotency_v1_immutable
BEFORE UPDATE ON instant_finalize_http_idempotency_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_reservation_v1');
END;

CREATE TRIGGER instant_finalize_http_idempotency_v1_no_delete
BEFORE DELETE ON instant_finalize_http_idempotency_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_reservation_v1');
END;

CREATE TRIGGER instant_finalize_reservation_assertions_v1_contract
BEFORE INSERT ON instant_finalize_reservation_assertions_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM instant_finalize_http_idempotency_v1 h
  JOIN instant_finalize_operations_v1 o ON o.operation_id = h.operation_id
  JOIN instant_finalize_requests_v1 r ON r.session_id = h.session_id
  JOIN instant_finalize_jobs_v1 j ON j.job_id = h.job_id
  WHERE h.organization_id = NEW.organization_id
    AND h.idempotency_key = NEW.idempotency_key
    AND h.operation_id = NEW.operation_id
    AND r.organization_id = h.organization_id
    AND r.request_sha256 = h.request_sha256
    AND o.session_id = r.session_id AND o.request_sha256 = r.request_sha256
    AND j.session_id = r.session_id AND j.request_sha256 = r.request_sha256
    AND j.generation = r.job_generation
)
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_reservation_v1');
END;

CREATE TRIGGER instant_finalize_reservation_assertions_v1_immutable
BEFORE UPDATE ON instant_finalize_reservation_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_reservation_v1');
END;

CREATE TRIGGER instant_finalize_reservation_assertions_v1_no_delete
BEFORE DELETE ON instant_finalize_reservation_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_reservation_v1');
END;

CREATE TABLE instant_finalize_dead_letters_v1 (
  session_id TEXT PRIMARY KEY NOT NULL
    REFERENCES instant_finalize_requests_v1(session_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  request_sha256 TEXT NOT NULL,
  attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 1 AND 65535),
  failure_class TEXT NOT NULL CHECK (
    failure_class IN ('dependency_pending', 'persistence', 'conflict')
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  FOREIGN KEY (request_sha256)
    REFERENCES instant_finalize_requests_v1(request_sha256) ON DELETE RESTRICT
);

CREATE TRIGGER instant_finalize_dead_letters_v1_contract
BEFORE INSERT ON instant_finalize_dead_letters_v1
WHEN NOT EXISTS (
  SELECT 1 FROM instant_finalize_requests_v1 r
  JOIN instant_finalize_jobs_v1 j ON j.job_id = r.job_id
    AND j.session_id = r.session_id AND j.state = 'cancelled'
  WHERE r.session_id = NEW.session_id AND r.organization_id = NEW.organization_id
    AND r.request_sha256 = NEW.request_sha256 AND r.state = 'dead_letter'
    AND r.reconcile_attempt_count = NEW.attempt_count
    AND r.last_failure_class = NEW.failure_class
    AND EXISTS (
      SELECT 1 FROM instant_finalize_operations_v1 o
      WHERE o.session_id = r.session_id AND o.result_state = 'dead_letter'
    )
    AND NOT EXISTS (
      SELECT 1 FROM instant_finalize_operations_v1 o
      WHERE o.session_id = r.session_id AND o.result_state <> 'dead_letter'
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_dead_letter_v1');
END;

CREATE TRIGGER instant_finalize_dead_letters_v1_immutable
BEFORE UPDATE ON instant_finalize_dead_letters_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_dead_letter_v1');
END;

CREATE TRIGGER instant_finalize_dead_letters_v1_no_delete
BEFORE DELETE ON instant_finalize_dead_letters_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_dead_letter_v1');
END;

CREATE TABLE instant_finalize_scheduler_v1 (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  cursor_session_id TEXT,
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991)
);
INSERT INTO instant_finalize_scheduler_v1(singleton, cursor_session_id, updated_at_ms)
VALUES(1, NULL, 0);

-- A publication assertion is deliberately last in the authority-fenced D1
-- batch. It observes every contingent write and aborts the transaction if the
-- upload, active integration, trusted probe, non-deleted video, object rows,
-- retained job, or any retry operation does not match exactly.
CREATE TABLE instant_finalize_publication_assertions_v1 (
  session_id TEXT PRIMARY KEY NOT NULL
    REFERENCES instant_finalize_requests_v1(session_id) ON DELETE RESTRICT,
  publication_id TEXT NOT NULL UNIQUE,
  asserted_at_ms INTEGER NOT NULL CHECK (asserted_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER instant_finalize_publication_assertions_v1_contract
BEFORE INSERT ON instant_finalize_publication_assertions_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM instant_finalize_requests_v1 r
  JOIN video_uploads u ON u.id = r.upload_id AND u.organization_id = r.organization_id
  JOIN r2_multipart_intents_v1 intent ON intent.upload_id = u.id
  JOIN storage_integrations i ON i.id = intent.integration_id
    AND i.organization_id = r.organization_id AND i.provider = 'r2' AND i.state = 'active'
    AND json_extract(i.capabilities_json, '$.multipart') = 1
  JOIN r2_multipart_sessions_v1 s ON s.upload_id = u.id
    AND s.object_key = u.source_object_key AND s.state = 'complete'
  JOIN r2_multipart_completions_v1 c ON c.upload_id = s.upload_id
  JOIN r2_multipart_verified_objects_v1 verified ON verified.upload_id = s.upload_id
    AND verified.provider_version = c.provider_version
    AND verified.provider_etag = c.provider_etag
    AND verified.bytes = c.bytes AND verified.checksum_sha256 = c.checksum_sha256
    AND verified.content_type = c.content_type
  JOIN videos v ON v.id = r.video_id AND v.organization_id = r.organization_id
    AND v.deleted_at_ms IS NULL AND v.state = 'ready'
    AND v.source_object_key = u.source_object_key AND v.playback_object_key = u.source_object_key
    AND v.duration_ms = c.duration_ms
  JOIN object_manifests m ON m.object_key = u.source_object_key
    AND m.organization_id = r.organization_id AND m.video_id = r.video_id
    AND m.role = 'source' AND m.object_version = u.source_version AND m.state = 'available'
    AND m.bytes = c.bytes AND m.checksum_sha256 = c.checksum_sha256
    AND m.content_type = c.content_type AND m.provider_etag = c.provider_etag
  JOIN storage_objects so ON so.integration_id = i.id AND so.object_key = u.source_object_key
    AND so.organization_id = r.organization_id AND so.video_id = r.video_id
    AND so.role = 'source' AND so.object_version = u.source_version AND so.state = 'available'
    AND so.deleted_at_ms IS NULL AND so.bytes = c.bytes
    AND so.checksum_sha256 = c.checksum_sha256 AND so.content_type = c.content_type
    AND so.provider_etag = c.provider_etag
  JOIN storage_governed_objects_v1 governed
    ON governed.organization_id = r.organization_id AND governed.object_key = u.source_object_key
    AND governed.role = 'source' AND governed.state = 'active'
    AND governed.malware_disposition = 'clean'
    AND governed.immutable_revision = u.source_version
    AND governed.bytes = c.bytes AND governed.checksum_sha256 = c.checksum_sha256
    AND governed.content_type = c.content_type
  JOIN instant_finalize_jobs_v1 j ON j.job_id = r.job_id AND j.session_id = r.session_id
    AND j.generation = r.job_generation AND j.request_sha256 = r.request_sha256
    AND j.state = 'published'
  WHERE r.session_id = NEW.session_id AND r.state = 'published'
    AND r.publication_id = NEW.publication_id
    AND r.playable_object_key = u.source_object_key AND r.distribution_eligible = 1
    AND u.state = 'complete' AND u.received_bytes = u.expected_bytes
    AND u.checksum_sha256 = c.checksum_sha256
    AND c.request_parts_sha256 = r.ordered_parts_sha256
    AND EXISTS (
      SELECT 1 FROM media_source_probes_v1 p
      WHERE p.organization_id = r.organization_id AND p.video_id = r.video_id
        AND p.source_version = u.source_version AND p.source_object_key = u.source_object_key
        AND p.source_checksum_sha256 = c.checksum_sha256 AND p.source_bytes = c.bytes
        AND p.source_content_type = c.content_type AND p.container = c.container
        AND p.video_codec = c.video_codec AND p.audio_codec = c.audio_codec
        AND p.duration_ms = c.duration_ms AND p.width = c.width AND p.height = c.height
        AND CAST((p.frame_rate_numerator * 1000) / p.frame_rate_denominator AS INTEGER)
          BETWEEN c.frame_rate_millihertz - 1 AND c.frame_rate_millihertz + 1
        AND p.trust = 'verified_native_probe' AND p.state = 'verified'
    )
    AND EXISTS (
      SELECT 1 FROM instant_finalize_operations_v1 o
      WHERE o.session_id = r.session_id AND o.result_state = 'published'
        AND o.publication_id = r.publication_id
    )
    AND NOT EXISTS (
      SELECT 1 FROM instant_finalize_operations_v1 o
      WHERE o.session_id = r.session_id
        AND (o.result_state <> 'published' OR o.publication_id <> r.publication_id)
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_publication_v1');
END;

CREATE TRIGGER instant_finalize_publication_assertions_v1_immutable
BEFORE UPDATE ON instant_finalize_publication_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_publication_v1');
END;

CREATE TRIGGER instant_finalize_publication_assertions_v1_no_delete
BEFORE DELETE ON instant_finalize_publication_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_instant_finalize_publication_v1');
END;
