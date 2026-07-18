UPDATE shared_videos
SET folder_id = ?2,
    sharing_mode = CASE WHEN ?2 IS NULL THEN 'organization' ELSE 'space' END,
    revision = revision + 1,
    last_operation_id = ?3
WHERE id = ?1
  AND organization_id = ?4
  AND revoked_at_ms IS NULL
  AND revision = ?5
  AND folder_id IS ?6
