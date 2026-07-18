SELECT
  operation.request_fingerprint,
  operation.reservation_digest,
  operation.state,
  operation.response_status,
  operation.result_digest,
  intent.reservation_digest AS intent_reservation_digest,
  audit.reservation_digest AS audit_reservation_digest
FROM legacy_api_execution_operations_v1 operation
LEFT JOIN legacy_api_execution_intents_v1 intent
  ON intent.reservation_digest = operation.reservation_digest
LEFT JOIN legacy_api_execution_audit_v1 audit
  ON audit.reservation_digest = operation.reservation_digest
WHERE operation.scope_digest = ?1
  AND operation.operation_id = ?2
  AND operation.idempotency_key_digest = ?3;
