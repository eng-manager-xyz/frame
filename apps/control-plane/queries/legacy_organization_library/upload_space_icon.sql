UPDATE spaces
SET legacy_icon_key = ?2, updated_at_ms = ?4,
    revision = revision + 1,
    legacy_organization_library_revision = legacy_organization_library_revision + 1,
    legacy_organization_library_last_operation_id = ?3,
    last_operation_id = ?3
WHERE id = ?1 AND deleted_at_ms IS NULL
