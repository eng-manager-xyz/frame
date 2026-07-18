UPDATE legacy_core_storage_operations_v1
SET result_binding_json = ?3,
    state = 'effect_pending'
WHERE operation_id = ?1
  AND request_digest = ?2
  AND state = 'claimed';
