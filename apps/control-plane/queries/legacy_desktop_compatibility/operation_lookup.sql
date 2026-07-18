SELECT
  operation.operation_id,
  operation.request_digest,
  operation.state,
  operation.organization_id,
  operation.target_id,
  receipt.status,
  receipt.result_kind,
  receipt.result_json,
  receipt.result_digest,
  (SELECT COUNT(*) FROM legacy_desktop_compatibility_audit_v1 audit
   WHERE audit.operation_id = operation.operation_id) AS audit_count
FROM legacy_desktop_compatibility_operations_v1 operation
LEFT JOIN legacy_desktop_compatibility_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
WHERE operation.source_operation_id = ?1
  AND operation.actor_id = ?2
  AND operation.idempotency_key_digest = ?3
LIMIT 2;
