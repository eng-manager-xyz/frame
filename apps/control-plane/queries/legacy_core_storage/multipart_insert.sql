INSERT INTO legacy_core_storage_multipart_v1(
  external_upload_id, provider_upload_id, initiate_operation_id,
  completion_operation_id, abort_operation_id, actor_id, organization_id,
  mapped_video_id, legacy_video_id, storage_integration_id, object_prefix, subpath,
  object_key, content_type, state, expected_bytes, parts_digest,
  created_at_ms, expires_at_ms, terminal_at_ms
) VALUES (
  ?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
  ?11, ?12, 'open', NULL, NULL, ?13, ?14, NULL
);
