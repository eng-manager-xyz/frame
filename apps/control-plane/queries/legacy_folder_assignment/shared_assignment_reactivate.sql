UPDATE shared_videos
SET revoked_at_ms = NULL,
    sharing_mode = CASE WHEN folder_id IS NULL THEN 'organization' ELSE 'space' END,
    shared_by_user_id = ?2,
    shared_at_ms = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE id = ?1
  AND organization_id = ?5
  AND revoked_at_ms IS NOT NULL
  AND revision = ?6
  AND folder_id IS ?7
