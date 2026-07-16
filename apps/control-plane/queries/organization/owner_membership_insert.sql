INSERT INTO organization_members(
  organization_id, user_id, role, state, has_pro_seat, created_at_ms,
  updated_at_ms, revision, authority_version, last_operation_id
) VALUES (?1, ?2, 'owner', 'active', ?3, ?4, ?4, 0, 0, ?5)
