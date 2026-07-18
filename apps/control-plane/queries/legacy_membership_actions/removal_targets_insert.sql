INSERT INTO legacy_membership_action_previous_members_v1(
  operation_id, user_id, legacy_user_id, legacy_member_id,
  mapped_member_id, role, state, revision
)
SELECT DISTINCT ?1, alias.user_id, alias.legacy_user_id, alias.legacy_member_id,
  alias.mapped_member_id, member.role, member.state, member.revision
FROM json_each(?2) requested
JOIN legacy_space_member_aliases_v1 alias
  ON alias.mapped_member_id = json_extract(requested.value, '$.mappedMemberId')
 AND alias.legacy_member_id = json_extract(requested.value, '$.legacyMemberId')
 AND alias.space_id = ?3
 AND alias.removed_at_ms IS NULL
JOIN space_members member
  ON member.space_id = alias.space_id
 AND member.user_id = alias.user_id
 AND member.state = 'active'
ORDER BY alias.user_id
