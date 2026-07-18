INSERT INTO legacy_collaboration_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'notification_bound', COUNT(*), MIN(COUNT(*), 100000)
FROM legacy_collaboration_notification_targets_v1
WHERE operation_id = ?1;
