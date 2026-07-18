UPDATE videos
SET folder_id = ?2,
    revision = revision + 1,
    last_operation_id = ?3,
    updated_at_ms = ?4
WHERE id = ?1
  AND organization_id = ?5
  AND deleted_at_ms IS NULL
  AND revision = ?6
  AND folder_id IS ?7
