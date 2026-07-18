SELECT
  operation_id,
  request_digest,
  result_binding_json,
  state,
  client_idempotency
FROM legacy_core_storage_operations_v1
WHERE operation_id = ?1
  AND actor_id = ?2
  AND source_operation_id = ?3
LIMIT 1;
