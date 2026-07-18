UPDATE legacy_core_storage_operations_v1
SET result_binding_json = ?3,
    state = 'complete',
    completed_at_ms = ?4
WHERE operation_id = ?1
  AND request_digest = ?2
  AND state IN ('claimed', 'effect_pending');
