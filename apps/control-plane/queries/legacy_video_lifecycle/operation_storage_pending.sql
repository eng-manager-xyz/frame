UPDATE legacy_video_lifecycle_operations_v1
SET state = 'storage_pending'
WHERE operation_id = ?1 AND state = 'claimed';
