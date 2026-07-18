INSERT INTO legacy_extension_instant_operations_v1(
  operation_id, source_operation_id, action, actor_id, organization_id,
  legacy_video_id, mapped_video_id, request_digest, applied, state,
  created_at_ms, completed_at_ms
) VALUES (
  ?1, 'cap-v1-00422c50f4d39053', 'create', ?2, ?3,
  ?4, ?5, ?6, 1, 'complete', ?7, ?7
);
