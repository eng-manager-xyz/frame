INSERT INTO legacy_mobile_session_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, ?2, ?3, COUNT(*)
FROM legacy_mobile_session_operations_v1 operation
JOIN legacy_mobile_session_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
JOIN legacy_mobile_session_audit_events_v1 audit
  ON audit.operation_id = operation.operation_id
WHERE operation.operation_id = ?1
  AND operation.action = ?4
  AND receipt.outcome = ?5
  AND audit.action = ?4
