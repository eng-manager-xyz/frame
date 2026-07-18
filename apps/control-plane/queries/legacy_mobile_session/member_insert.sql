INSERT INTO organization_members(
  organization_id, user_id, role, state, has_pro_seat,
  created_at_ms, updated_at_ms, revision
) VALUES(?1, ?2, 'owner', 'active', 0, ?3, ?3, 0)
