UPDATE spaces
SET name = ?4,
    is_public = ?5,
    settings_json = ?6,
    updated_at_ms = ?7,
    revision = revision + 1,
    last_operation_id = ?8
WHERE id = ?1
  AND organization_id = ?2
  AND revision = ?3
  AND deleted_at_ms IS NULL
