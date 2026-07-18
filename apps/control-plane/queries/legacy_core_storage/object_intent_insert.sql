INSERT INTO legacy_core_storage_object_intents_v1(
  intent_id, object_key, operation_id, actor_id, organization_id, mapped_video_id,
  legacy_video_id, storage_integration_id, content_type, object_role, method,
  state, created_at_ms, observed_at_ms
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
  'capability_issued', ?12, NULL
);
