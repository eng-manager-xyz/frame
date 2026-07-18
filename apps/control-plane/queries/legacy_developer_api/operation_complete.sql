UPDATE legacy_developer_api_operations_v1
SET state = 'complete', completed_at_ms = ?2
WHERE operation_id = ?1 AND state IN ('claimed','effect_pending')
