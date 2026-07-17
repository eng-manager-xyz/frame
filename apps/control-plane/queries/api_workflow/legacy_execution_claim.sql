INSERT OR IGNORE INTO legacy_api_execution_operations_v1(
  scope_digest,
  operation_id,
  idempotency_key_digest,
  request_fingerprint,
  reservation_digest,
  state,
  response_status,
  result_digest,
  created_at_ms,
  completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, ?6, NULL)
RETURNING reservation_digest;
