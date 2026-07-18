UPDATE video_uploads
SET received_bytes = ?4,
    expected_bytes = ?5,
    updated_at_ms = ?6,
    last_operation_id = ?7
WHERE id = (
  SELECT instant.upload_id
  FROM legacy_extension_instant_recordings_v1 instant
  WHERE instant.legacy_video_id = ?1
    AND instant.mapped_video_id = ?2
    AND instant.actor_id = ?3
    AND instant.lifecycle_state = 'active'
)
  AND updated_at_ms <= ?6
  AND received_bytes <= ?4
  AND expected_bytes <= ?5
  AND (received_bytes <> ?4 OR expected_bytes <> ?5 OR updated_at_ms <> ?6);
