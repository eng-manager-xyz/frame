SELECT
  operation_id,
  request_digest,
  result_binding_json,
  state,
  client_idempotency
FROM legacy_core_storage_operations_v1
WHERE source_operation_id = ?1
  AND actor_id = ?2
  AND idempotency_key_digest = ?3
LIMIT 1;
