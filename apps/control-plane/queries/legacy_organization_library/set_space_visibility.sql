UPDATE spaces
SET is_public = COALESCE(?2, is_public),
    settings_json = CASE
      WHEN ?3 IS NULL THEN settings_json
      ELSE json_patch(settings_json, json_object('publicPage', json(?3)))
    END,
    updated_at_ms = ?5,
    revision = revision + 1,
    legacy_organization_library_revision = legacy_organization_library_revision + 1,
    legacy_organization_library_last_operation_id = ?4,
    last_operation_id = ?4
WHERE id = ?1 AND deleted_at_ms IS NULL
