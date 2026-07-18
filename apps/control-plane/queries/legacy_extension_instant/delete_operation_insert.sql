INSERT INTO legacy_extension_instant_operations_v1(
  operation_id, source_operation_id, action, actor_id, organization_id,
  legacy_video_id, mapped_video_id, request_digest, applied, state,
  created_at_ms, completed_at_ms
) VALUES (
  ?1, 'cap-v1-8fd4741d6e52465e', 'delete', ?2, ?3,
  ?4, ?5, ?6, 0, 'pending_storage', ?7, NULL
);
