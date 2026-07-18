INSERT INTO legacy_video_lifecycle_operations_v1(
  operation_id, source_operation_id, action, actor_id, organization_id,
  mapped_video_id, legacy_video_id, request_key_digest, request_digest,
  destination_mapped_video_id, destination_legacy_video_id,
  source_prefix, destination_prefix, result_json, state, created_at_ms
) VALUES (
  ?1, ?2, ?3, ?4, ?5,
  ?6, ?7, ?8, ?9,
  ?10, ?11,
  ?12, ?13, ?14, ?15, ?16
)
;
