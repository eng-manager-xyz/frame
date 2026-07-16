UPDATE organizations
SET name = ?2,
    settings_json = ?3,
    updated_at_ms = ?4,
    revision = revision + 1,
    last_operation_id = ?5
WHERE id = ?1
