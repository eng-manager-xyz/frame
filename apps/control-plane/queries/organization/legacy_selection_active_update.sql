UPDATE users
SET active_organization_id = ?2,
    organization_preference_revision = organization_preference_revision + 1,
    organization_last_operation_id = ?3,
    updated_at_ms = ?4
WHERE id = ?1
