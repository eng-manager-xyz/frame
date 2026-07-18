SELECT
  external_upload_id,
  provider_upload_id,
  initiate_operation_id,
  completion_operation_id,
  abort_operation_id,
  actor_id,
  organization_id,
  mapped_video_id,
  legacy_video_id,
  storage_integration_id,
  object_prefix,
  subpath,
  object_key,
  content_type,
  state,
  expected_bytes,
  parts_digest,
  created_at_ms,
  expires_at_ms,
  terminal_at_ms
FROM legacy_core_storage_multipart_v1
WHERE external_upload_id = ?1
  AND actor_id = ?2
LIMIT 1;
