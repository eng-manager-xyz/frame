UPDATE legacy_desktop_video_uploads_v1
SET uploaded = ?2,
    total = ?3,
    updated_at_ms = ?4,
    revision = revision + 1,
    last_operation_id = ?5
WHERE video_id = ?1
  AND revision = ?6
  AND updated_at_ms <= ?4;
