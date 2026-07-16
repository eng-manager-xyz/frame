UPDATE organization_members
SET role = 'admin',
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = ?3,
    last_operation_id = ?4
WHERE organization_id = ?1 AND user_id = ?2 AND role = 'owner' AND state = 'active'
