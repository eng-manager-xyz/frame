INSERT INTO legacy_api_execution_intents_v1(
  reservation_digest,
  scope_digest,
  operation_id,
  idempotency_key_digest,
  request_fingerprint,
  created_at_ms
)
SELECT ?4, ?1, ?2, ?3, ?5, ?6
WHERE EXISTS (
  SELECT 1
  FROM legacy_api_execution_operations_v1 operation
  WHERE operation.scope_digest = ?1
    AND operation.operation_id = ?2
    AND operation.idempotency_key_digest = ?3
    AND operation.reservation_digest = ?4
    AND operation.request_fingerprint = ?5
    AND operation.state = 'pending'
)
ON CONFLICT(reservation_digest) DO NOTHING
RETURNING reservation_digest;
