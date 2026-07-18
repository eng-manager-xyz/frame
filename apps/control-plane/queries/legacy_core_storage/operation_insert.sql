INSERT INTO legacy_core_storage_operations_v1(
  operation_id, source_operation_id, operation_kind, actor_id,
  organization_id, mapped_video_id, legacy_video_id,
  idempotency_key_digest, client_idempotency, request_digest,
  result_binding_json, state, created_at_ms, completed_at_ms
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'claimed', ?12, NULL
);
