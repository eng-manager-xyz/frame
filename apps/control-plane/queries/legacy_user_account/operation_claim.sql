INSERT INTO legacy_user_account_operations_v1(
  operation_id, actor_id, action, idempotency_key_digest,
  request_digest, state, created_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6);
