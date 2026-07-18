INSERT INTO legacy_mobile_upload_operations_v1(
  operation_id, source_operation_id, operation_kind, actor_id,
  organization_id, mapped_video_id, legacy_video_id, request_digest,
  state, created_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10);
