UPDATE organization_members
SET role = ?3, updated_at_ms = ?4,
    revision = revision + 1, authority_version = authority_version + 1,
    last_operation_id = ?5
WHERE organization_id = ?1 AND user_id = ?2 AND state = 'active'
