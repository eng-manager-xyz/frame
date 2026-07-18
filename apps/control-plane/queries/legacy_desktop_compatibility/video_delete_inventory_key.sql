INSERT OR IGNORE INTO legacy_desktop_video_delete_objects_v1(
  operation_id, object_key, state, deleted_at_ms
) VALUES (?1, ?2, 'pending', NULL);
