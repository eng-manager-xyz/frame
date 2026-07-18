UPDATE organization_members
SET role = ?4,
    state = ?5,
    has_pro_seat = ?6,
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = ?7,
    last_operation_id = ?8
WHERE organization_id = ?1
  AND user_id = ?2
  AND revision = ?3
  AND role <> 'owner'
  AND ?4 <> 'owner'
