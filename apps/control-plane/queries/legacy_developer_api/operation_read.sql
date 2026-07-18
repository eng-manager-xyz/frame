SELECT operation.operation_id, operation.request_digest, operation.state,
       receipt.status, receipt.result_kind, receipt.result_json
FROM legacy_developer_api_operations_v1 AS operation
LEFT JOIN legacy_developer_api_receipts_v1 AS receipt
  ON receipt.operation_id = operation.operation_id
WHERE operation.source_operation_id = ?1 AND operation.app_id = ?2
  AND operation.idempotency_key_digest = ?3
LIMIT 1
