UPDATE legacy_desktop_compatibility_operations_v1
SET state = 'effect_pending'
WHERE operation_id = ?1 AND state = 'claimed';
