UPDATE imported_videos
SET state = ?4,
    event_sequence = ?3,
    event_fingerprint = ?5,
    revision = revision + 1,
    updated_at_ms = ?6,
    error_class = ?7,
    last_operation_id = ?8
WHERE id = ?1 AND organization_id = ?2 AND event_sequence + 1 = ?3
