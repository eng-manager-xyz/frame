UPDATE organizations
SET owner_id = ?2,
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = ?3,
    last_operation_id = ?4
WHERE id = ?1
