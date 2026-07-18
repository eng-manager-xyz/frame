UPDATE video_uploads
SET state = 'aborted',
    updated_at_ms = MAX(updated_at_ms, ?3),
    revision = revision + 1,
    event_sequence = event_sequence + 1,
    event_fingerprint = ?4,
    last_operation_id = ?5
WHERE id = ?1
  AND organization_id = ?2
  AND state IN ('initiated', 'uploading', 'finalizing', 'failed');
