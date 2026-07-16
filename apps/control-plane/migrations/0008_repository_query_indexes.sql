PRAGMA foreign_keys = ON;

-- Keyset pagination is ordered by both timestamp and opaque identifier. The
-- partial predicate matches the aggregate repository's active-video scope.
CREATE INDEX videos_org_active_page_idx
  ON videos(organization_id, created_at_ms DESC, id DESC)
  WHERE deleted_at_ms IS NULL;

-- Organization snapshots count non-terminal uploads by tenant. The original
-- upload index starts with video_id and would otherwise scan every upload.
CREATE INDEX video_uploads_org_state_idx
  ON video_uploads(organization_id, state);

-- A repository operation is a transient command envelope. Its BEFORE trigger
-- performs the reservation, revision-fenced aggregate update, outbox insert,
-- and stored response as one SQLite statement. Each changes() guard raises and
-- rolls back the statement if any expected row is absent. The final
-- RAISE(IGNORE) suppresses the transient envelope insert without rolling back
-- the trigger's prior effects, so this table remains empty after success.
CREATE TABLE repository_video_title_operations (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 128),
  request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
  reservation_id TEXT NOT NULL CHECK (length(reservation_id) = 36),
  outbox_id TEXT NOT NULL CHECK (length(outbox_id) = 36),
  deduplication_key TEXT NOT NULL,
  expected_revision INTEGER NOT NULL CHECK (expected_revision BETWEEN 0 AND 9007199254740990),
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 160),
  response_json TEXT NOT NULL CHECK (json_valid(response_json)),
  payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
  now_ms INTEGER NOT NULL CHECK (now_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > now_ms),
  CHECK (deduplication_key = 'repository-video-title:' || organization_id || ':' || idempotency_key),
  CHECK (operation_id <> reservation_id AND operation_id <> outbox_id AND reservation_id <> outbox_id)
);

CREATE TRIGGER repository_video_title_apply
BEFORE INSERT ON repository_video_title_operations
BEGIN
  SELECT CASE WHEN
    NEW.response_json = json_object(
      'schema_version', 1,
      'video_id', NEW.video_id,
      'title', NEW.title,
      'revision', NEW.expected_revision + 1
    )
    AND NEW.payload_json = NEW.response_json
  THEN 1 ELSE RAISE(ABORT, 'repository video title envelope invalid') END;

  INSERT INTO command_idempotency(
    organization_id,
    idempotency_key,
    command_type,
    request_digest,
    response_status,
    response_json,
    created_at_ms,
    expires_at_ms,
    reservation_id
  )
  SELECT NEW.organization_id,
         NEW.idempotency_key,
         'repository_video_title_v1',
         NEW.request_digest,
         NULL,
         NULL,
         NEW.now_ms,
         NEW.expires_at_ms,
         NEW.reservation_id
  FROM videos v
  JOIN organizations o
    ON o.id = v.organization_id
   AND o.status = 'active'
  JOIN organization_members m
    ON m.organization_id = v.organization_id
   AND m.user_id = NEW.actor_id
   AND m.state = 'active'
  WHERE v.id = NEW.video_id
    AND v.organization_id = NEW.organization_id
    AND v.revision = NEW.expected_revision
    AND v.deleted_at_ms IS NULL
    AND (
      m.role IN ('owner', 'admin')
      OR (
        m.role = 'member'
        AND (
          v.owner_id = NEW.actor_id
          OR EXISTS (
            SELECT 1
            FROM space_videos sv
            JOIN spaces s
              ON s.id = sv.space_id
             AND s.organization_id = v.organization_id
             AND s.deleted_at_ms IS NULL
            JOIN space_members sm
              ON sm.space_id = s.id
             AND sm.user_id = NEW.actor_id
             AND sm.role = 'manager'
            WHERE sv.video_id = v.id
          )
        )
      )
    )
    AND NOT EXISTS (
      SELECT 1
      FROM command_idempotency c
      WHERE c.organization_id = NEW.organization_id
        AND c.idempotency_key = NEW.idempotency_key
    );
  SELECT CASE WHEN changes() = 1
    THEN 1 ELSE RAISE(ABORT, 'repository video title reservation rejected') END;

  UPDATE videos
  SET title = NEW.title,
      updated_at_ms = NEW.now_ms,
      revision = revision + 1
  WHERE id = NEW.video_id
    AND organization_id = NEW.organization_id
    AND revision = NEW.expected_revision
    AND deleted_at_ms IS NULL
    AND EXISTS (
      SELECT 1
      FROM command_idempotency c
      WHERE c.organization_id = NEW.organization_id
        AND c.idempotency_key = NEW.idempotency_key
        AND c.request_digest = NEW.request_digest
        AND c.reservation_id = NEW.reservation_id
        AND c.response_status IS NULL
        AND c.response_json IS NULL
    );
  SELECT CASE WHEN changes() = 1
    THEN 1 ELSE RAISE(ABORT, 'repository video title aggregate rejected') END;

  INSERT INTO outbox_events(
    id,
    organization_id,
    aggregate_type,
    aggregate_id,
    event_type,
    deduplication_key,
    payload_json,
    state,
    attempt,
    available_at_ms,
    created_at_ms
  ) VALUES (
    NEW.outbox_id,
    NEW.organization_id,
    'video',
    NEW.video_id,
    'video.title_updated.v1',
    NEW.deduplication_key,
    NEW.payload_json,
    'pending',
    0,
    NEW.now_ms,
    NEW.now_ms
  );
  SELECT CASE WHEN changes() = 1
    THEN 1 ELSE RAISE(ABORT, 'repository video title outbox rejected') END;

  UPDATE command_idempotency
  SET response_status = 200,
      response_json = NEW.response_json
  WHERE organization_id = NEW.organization_id
    AND idempotency_key = NEW.idempotency_key
    AND request_digest = NEW.request_digest
    AND reservation_id = NEW.reservation_id
    AND response_status IS NULL
    AND response_json IS NULL
    AND EXISTS (
      SELECT 1
      FROM videos v
      WHERE v.id = NEW.video_id
        AND v.organization_id = NEW.organization_id
        AND v.revision = NEW.expected_revision + 1
        AND v.title = NEW.title
        AND v.deleted_at_ms IS NULL
    )
    AND EXISTS (
      SELECT 1
      FROM outbox_events e
      WHERE e.organization_id = NEW.organization_id
        AND e.aggregate_type = 'video'
        AND e.aggregate_id = NEW.video_id
        AND e.event_type = 'video.title_updated.v1'
        AND e.deduplication_key = NEW.deduplication_key
        AND e.payload_json = NEW.payload_json
    );
  SELECT CASE WHEN changes() = 1
    THEN 1 ELSE RAISE(ABORT, 'repository video title response rejected') END;
  SELECT RAISE(IGNORE);
END;
