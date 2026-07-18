INSERT INTO legacy_membership_action_final_members_v1(
  operation_id, user_id, legacy_user_id, legacy_member_id,
  mapped_member_id, role, ordinal
)
SELECT ?1, parsed.user_id, parsed.legacy_user_id, parsed.legacy_member_id,
  parsed.mapped_member_id, parsed.role, parsed.ordinal
FROM (
  SELECT
    json_extract(member.value, '$.userId') AS user_id,
    json_extract(member.value, '$.legacyUserId') AS legacy_user_id,
    json_extract(member.value, '$.legacyMemberId') AS legacy_member_id,
    json_extract(member.value, '$.mappedMemberId') AS mapped_member_id,
    CASE json_extract(member.value, '$.role')
      WHEN 'manager' THEN 'manager'
      WHEN 'viewer' THEN 'viewer'
      ELSE 'invalid'
    END AS role,
    CAST(member.key AS INTEGER) AS ordinal,
    ROW_NUMBER() OVER (
      PARTITION BY json_extract(member.value, '$.userId')
      ORDER BY CAST(member.key AS INTEGER)
    ) AS user_ordinal
  FROM json_each(?2) member
) parsed
WHERE parsed.user_ordinal = 1
ORDER BY parsed.ordinal
