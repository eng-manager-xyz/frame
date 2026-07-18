UPDATE video_uploads
SET received_bytes = ?3,
    expected_bytes = ?4,
    updated_at_ms = ?5,
    last_operation_id = ?6
WHERE id = ?2
  AND video_id = ?1
  AND updated_at_ms <= ?5;
