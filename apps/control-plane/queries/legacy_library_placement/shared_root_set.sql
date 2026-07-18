UPDATE shared_videos
SET folder_id = NULL,
    sharing_mode = 'organization',
    revision = revision + 1,
    last_operation_id = ?2
WHERE id = ?1
  AND organization_id = ?3
  AND revoked_at_ms IS NULL
  AND revision = ?4
  AND folder_id IS ?5
  AND sharing_mode = ?6
