INSERT INTO legacy_membership_action_previous_members_v1(
  operation_id, user_id, legacy_user_id, legacy_member_id,
  mapped_member_id, role, state, revision
)
SELECT ?1, member.user_id, alias.legacy_user_id, alias.legacy_member_id,
  alias.mapped_member_id, member.role, member.state, member.revision
FROM space_members member
JOIN legacy_space_member_aliases_v1 alias
  ON alias.space_id = member.space_id
 AND alias.user_id = member.user_id
 AND alias.removed_at_ms IS NULL
WHERE member.space_id = ?2
ORDER BY member.user_id
LIMIT 100001
