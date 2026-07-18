UPDATE organizations
SET settings_json = CASE
      WHEN ?2 IS NULL THEN settings_json
      ELSE json_patch(settings_json, ?2)
    END,
    legacy_shareable_link_icon_key = CASE
      WHEN ?3 = 0 THEN legacy_shareable_link_icon_key
      ELSE ?4
    END,
    updated_at_ms = ?6,
    revision = revision + 1,
    legacy_organization_library_revision = legacy_organization_library_revision + 1,
    legacy_organization_library_last_operation_id = ?5,
    last_operation_id = ?5
WHERE id = ?1 AND status = 'active' AND tombstoned_at_ms IS NULL
