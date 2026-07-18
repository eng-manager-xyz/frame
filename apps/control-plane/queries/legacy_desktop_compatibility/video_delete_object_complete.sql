UPDATE legacy_desktop_video_delete_objects_v1
SET state = 'deleted', deleted_at_ms = ?3
WHERE operation_id = ?1 AND object_key = ?2 AND state = 'pending';
