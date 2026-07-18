UPDATE folders
SET settings_json = json_patch(
      settings_json,
      json_object(
        'publicPage',
        json_object('logoUrl', ?2, 'logoMode', CASE WHEN ?2 IS NULL THEN 'cap' ELSE 'custom' END)
      )
    ),
    updated_at_ms = ?4,
    revision = revision + 1,
    legacy_organization_library_revision = legacy_organization_library_revision + 1,
    legacy_organization_library_last_operation_id = ?3,
    last_operation_id = ?3
WHERE id = ?1 AND deleted_at_ms IS NULL
