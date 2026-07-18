UPDATE folders
SET name = ?5,
    legacy_name = CASE WHEN legacy_name IS NULL THEN NULL ELSE ?5 END,
    is_public = ?6,
    settings_json = ?7,
    updated_at_ms = ?8,
    revision = revision + 1,
    last_operation_id = ?9
WHERE id = ?1
  AND organization_id = ?2
  AND space_id = ?3
  AND revision = ?4
  AND deleted_at_ms IS NULL
