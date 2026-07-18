DELETE FROM shared_videos
WHERE id = ?1
  AND organization_id = ?2
  AND revoked_at_ms IS NULL
  AND revision = ?3
