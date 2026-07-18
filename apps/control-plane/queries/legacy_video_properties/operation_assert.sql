INSERT INTO legacy_video_property_assertions_v1 (
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'operation', 1, COUNT(*)
FROM legacy_video_property_operations_v1
WHERE operation_id = ?1
  AND source_operation_id = ?2
  AND operation_kind = ?3
  AND principal_digest = ?4
  AND video_id = ?5
  AND legacy_video_id_digest = ?6
  AND idempotency_key_digest = ?7
  AND request_digest = ?8
  AND state = 'claimed';
