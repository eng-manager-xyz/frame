INSERT INTO legacy_collaboration_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'user_alias', 1, COUNT(*)
FROM legacy_collaboration_user_aliases_v1
WHERE legacy_user_id = ?2 AND mapped_user_id = ?3;
