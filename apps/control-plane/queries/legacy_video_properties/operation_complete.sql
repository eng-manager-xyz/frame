UPDATE legacy_video_property_operations_v1
SET state = 'complete', completed_at_ms = ?2
WHERE operation_id = ?1 AND state = 'claimed';
