INSERT INTO legacy_developer_action_operations_v1(
  operation_id, actor_id, action, idempotency_key_digest, request_digest,
  state, created_at_ms, completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, 'claimed', ?6, NULL)
