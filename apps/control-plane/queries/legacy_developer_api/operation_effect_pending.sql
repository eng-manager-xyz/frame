UPDATE legacy_developer_api_operations_v1
SET state = 'effect_pending'
WHERE operation_id = ?1 AND state = 'claimed'
