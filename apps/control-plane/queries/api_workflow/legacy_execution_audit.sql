INSERT INTO legacy_api_execution_audit_v1(
  audit_id,
  reservation_digest,
  scope_digest,
  operation_id,
  audit_action,
  outcome,
  correlation_digest,
  result_digest,
  occurred_at_ms
)
SELECT ?9, ?4, ?1, ?2, ?10, 'accepted', ?11, ?7, ?8
FROM legacy_api_execution_operations_v1 operation
WHERE operation.scope_digest = ?1
  AND operation.operation_id = ?2
  AND operation.idempotency_key_digest = ?3
  AND operation.reservation_digest = ?4
  AND operation.request_fingerprint = ?5
  AND operation.state = 'complete'
  AND operation.response_status = ?6
  AND operation.result_digest = ?7
  AND operation.completed_at_ms = ?8
  AND EXISTS (
    SELECT 1
    FROM legacy_api_execution_intents_v1 intent
    WHERE intent.reservation_digest = ?4
      AND intent.request_fingerprint = ?5
  )
ON CONFLICT(audit_id) DO NOTHING
RETURNING audit_id;
