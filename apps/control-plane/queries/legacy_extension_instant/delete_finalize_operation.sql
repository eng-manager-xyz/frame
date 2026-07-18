UPDATE legacy_extension_instant_operations_v1
SET applied = 1,
    state = 'complete',
    completed_at_ms = ?2
WHERE operation_id = ?1
  AND action = 'delete'
  AND state = 'pending_storage';
