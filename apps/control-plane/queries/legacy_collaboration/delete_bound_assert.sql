INSERT INTO legacy_collaboration_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'delete_bound', COUNT(*), MIN(COUNT(*), 100000)
FROM legacy_collaboration_delete_targets_v1
WHERE operation_id = ?1;
