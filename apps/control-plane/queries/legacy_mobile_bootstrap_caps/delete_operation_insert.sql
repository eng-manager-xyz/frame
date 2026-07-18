INSERT INTO legacy_mobile_cap_delete_operations_v1(
  operation_id, actor_id, mapped_video_id, legacy_video_id, object_prefix,
  state, created_at_ms, completed_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, 'storage_pending', ?6, NULL);
