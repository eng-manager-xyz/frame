UPDATE shared_videos
SET revoked_at_ms = ?2,
    revision = revision + 1,
    last_operation_id = ?3
WHERE id = ?1
  AND organization_id = ?4
  AND revoked_at_ms IS NULL
  AND revision = ?5
