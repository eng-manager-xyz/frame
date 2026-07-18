UPDATE legacy_video_lifecycle_operations_v1
SET state = 'complete',
    result_json = COALESCE(result_json, ?2),
    completed_at_ms = ?3
WHERE operation_id = ?1 AND state IN ('claimed', 'storage_pending');
