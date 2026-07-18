INSERT INTO legacy_developer_api_operations_v1(
  operation_id, source_operation_id, app_id, target_id,
  idempotency_key_digest, request_digest, state, created_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'claimed', ?7)
