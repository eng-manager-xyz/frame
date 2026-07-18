UPDATE legacy_desktop_personal_storage_integrations_v1
SET active = 0,
    updated_at_ms = ?2,
    revision = revision + 1,
    last_operation_id = ?3
WHERE owner_user_id = ?1;
