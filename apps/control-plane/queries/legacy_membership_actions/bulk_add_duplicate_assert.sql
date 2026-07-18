INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'bulk_add_duplicate', 0,
  COUNT(*) - COUNT(DISTINCT json_extract(requested.value, '$.userId'))
FROM json_each(?2) requested
WHERE NOT EXISTS (
  SELECT 1 FROM space_members member
  WHERE member.space_id = ?3
    AND member.user_id = json_extract(requested.value, '$.userId')
)
