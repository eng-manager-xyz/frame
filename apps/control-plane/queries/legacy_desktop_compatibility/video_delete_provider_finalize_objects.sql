UPDATE storage_objects
SET state = 'deleted', deleted_at_ms = ?2, updated_at_ms = ?2,
    revision = revision + 1, last_operation_id = ?1
WHERE object_key IN (
  SELECT object_key FROM legacy_desktop_video_delete_objects_v1 WHERE operation_id = ?1
);
