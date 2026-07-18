UPDATE organization_members
SET state = 'removed', has_pro_seat = 0, updated_at_ms = ?3,
    revision = revision + 1, authority_version = authority_version + 1,
    last_operation_id = ?4
WHERE organization_id = ?1 AND user_id = ?2 AND state = 'active'
