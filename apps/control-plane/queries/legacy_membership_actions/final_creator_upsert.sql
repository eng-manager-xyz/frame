INSERT INTO legacy_membership_action_final_members_v1(
  operation_id, user_id, legacy_user_id, legacy_member_id,
  mapped_member_id, role, ordinal
)
SELECT ?1, ?2, alias.legacy_user_id, ?4, ?5, 'manager', 500
FROM legacy_space_member_aliases_v1 alias
WHERE alias.space_id = ?3 AND alias.user_id = ?2 AND alias.removed_at_ms IS NULL
ON CONFLICT(operation_id, user_id) DO UPDATE SET role = 'manager'
