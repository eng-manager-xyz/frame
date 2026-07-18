INSERT INTO legacy_collaboration_operations_v1(
  operation_id, organization_id, actor_id, action,
  idempotency_key_digest, request_digest, state, created_at_ms, completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'claimed', ?7, NULL);
