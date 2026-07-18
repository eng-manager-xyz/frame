UPDATE storage_objects
SET state = 'deleting', updated_at_ms = ?3, revision = revision + 1,
    last_operation_id = ?1
WHERE video_id = ?2 AND state NOT IN ('deleted','missing');
