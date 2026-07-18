INSERT INTO legacy_desktop_compatibility_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'claim', 1, COUNT(*)
FROM legacy_desktop_compatibility_operations_v1
WHERE operation_id = ?1
  AND source_operation_id = ?2
  AND operation_kind = ?3
  AND actor_id = ?4
  AND organization_id IS ?5
  AND target_id IS ?6
  AND idempotency_key_digest = ?7
  AND request_digest = ?8
  AND state = 'claimed';
