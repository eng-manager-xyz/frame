INSERT INTO organization_members(
  organization_id, user_id, role, state, has_pro_seat,
  created_at_ms, updated_at_ms, revision, authority_version, last_operation_id
)
SELECT ?1, ?2, ?3, 'active', 0, ?4, ?4, 0, 0, ?5
WHERE ?6 = 0;
