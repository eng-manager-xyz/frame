INSERT INTO legacy_mobile_cap_delete_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'cleanup', 1, COUNT(*)
FROM legacy_mobile_cap_delete_operations_v1
WHERE operation_id = ?1
  AND state = 'complete'
  AND completed_at_ms = ?2;
