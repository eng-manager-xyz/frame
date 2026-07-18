PRAGMA foreign_keys = ON;

-- Migration 0011 adds ordered, checksummed business outbox fields. The
-- aggregate repository title command was introduced earlier as an atomic
-- transient envelope, so replace only that trigger after all larger authority
-- migrations have applied. Keeping this compatibility trigger separate also
-- stays below D1's per-migration compound-statement limit.
DROP TRIGGER repository_video_title_apply;
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
    AND NEW.payload_checksum IS NOT NULL
    AND length(NEW.payload_checksum) = 64
    AND NEW.payload_checksum NOT GLOB '*[^0-9a-f]*'
    AND NEW.event_fingerprint = 'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9'
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
    created_at_ms,
    event_sequence,
    event_fingerprint,
    payload_schema_version,
    payload_checksum,
    revision
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
    NEW.now_ms,
    0,
    NEW.event_fingerprint,
    1,
    NEW.payload_checksum,
    0
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
        AND e.event_sequence = 0
        AND e.event_fingerprint = NEW.event_fingerprint
        AND e.payload_checksum = NEW.payload_checksum
    );
  SELECT CASE WHEN changes() = 1
    THEN 1 ELSE RAISE(ABORT, 'repository video title response rejected') END;
  SELECT RAISE(IGNORE);
END;
