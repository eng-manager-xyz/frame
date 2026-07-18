INSERT INTO legacy_video_property_operations_v1 (
  operation_id, source_operation_id, operation_kind, principal_digest,
  video_id, legacy_video_id_digest, idempotency_key_digest, request_digest,
  state, created_at_ms, completed_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'claimed', ?9, NULL);
