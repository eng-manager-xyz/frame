UPDATE object_manifests
SET state = 'deleted', updated_at_ms = ?2
WHERE object_key IN (
  SELECT object_key FROM legacy_desktop_video_delete_objects_v1 WHERE operation_id = ?1
);
