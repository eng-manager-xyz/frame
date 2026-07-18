SELECT DISTINCT
  alias.mapped_member_id,
  alias.legacy_member_id,
  alias.legacy_user_id,
  alias.space_id,
  alias.user_id,
  alias.removed_at_ms,
  member.role,
  member.state,
  member.revision
FROM json_each(?1) requested
JOIN legacy_space_member_aliases_v1 alias
  ON alias.mapped_member_id = json_extract(requested.value, '$.mappedMemberId')
 AND alias.legacy_member_id = json_extract(requested.value, '$.legacyMemberId')
LEFT JOIN space_members member
  ON member.space_id = alias.space_id AND member.user_id = alias.user_id
ORDER BY alias.mapped_member_id
LIMIT 501
