INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'member_targets', 0, COUNT(DISTINCT alias.mapped_member_id)
FROM json_each(?2) requested
JOIN legacy_space_member_aliases_v1 alias
  ON alias.mapped_member_id = json_extract(requested.value, '$.mappedMemberId')
 AND alias.legacy_member_id = json_extract(requested.value, '$.legacyMemberId')
 AND alias.removed_at_ms IS NULL
JOIN space_members member
  ON member.space_id = alias.space_id
 AND member.user_id = alias.user_id
 AND member.state = 'active'
