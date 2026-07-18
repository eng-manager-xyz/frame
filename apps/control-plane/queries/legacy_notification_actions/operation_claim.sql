INSERT INTO legacy_notification_action_operations_v1(
  operation_id, tenant_kind, tenant_id, organization_id, actor_id, action,
  idempotency_key_digest, request_digest, state, created_at_ms, completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'claimed', ?9, NULL)
