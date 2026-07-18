INSERT INTO space_members(
  space_id, user_id, role, created_at_ms, updated_at_ms,
  state, revision, last_operation_id
)
SELECT ?2, final.user_id, final.role, ?3, ?3, 'active', 0, ?1
FROM legacy_membership_action_final_members_v1 final
WHERE final.operation_id = ?1
