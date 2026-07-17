UPDATE legacy_api_execution_operations_v1
SET state = 'complete',
    response_status = ?6,
    result_digest = ?7,
    completed_at_ms = ?8
WHERE scope_digest = ?1
  AND operation_id = ?2
  AND idempotency_key_digest = ?3
  AND reservation_digest = ?4
  AND request_fingerprint = ?5
  AND state = 'pending'
  AND EXISTS (
    SELECT 1
    FROM legacy_api_execution_intents_v1 intent
    WHERE intent.reservation_digest = ?4
      AND intent.scope_digest = ?1
      AND intent.operation_id = ?2
      AND intent.idempotency_key_digest = ?3
      AND intent.request_fingerprint = ?5
  )
RETURNING reservation_digest;
