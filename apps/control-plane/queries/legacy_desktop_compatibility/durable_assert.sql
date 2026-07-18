INSERT INTO legacy_desktop_compatibility_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'durable', 1,
  CASE WHEN
    EXISTS (
      SELECT 1 FROM legacy_desktop_compatibility_operations_v1 operation
      WHERE operation.operation_id = ?1
        AND operation.state = 'complete' AND operation.completed_at_ms = ?2
    )
    AND EXISTS (
      SELECT 1 FROM legacy_desktop_compatibility_receipts_v1 receipt
      WHERE receipt.operation_id = ?1 AND receipt.result_digest = ?3
    )
    AND EXISTS (
      SELECT 1 FROM legacy_desktop_compatibility_audit_v1 audit
      WHERE audit.operation_id = ?1 AND audit.result_digest = ?3
    )
  THEN 1 ELSE 0 END;
