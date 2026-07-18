INSERT INTO legacy_invite_lifecycle_operations_v1(
  operation_id, actor_id, organization_id, legacy_invite_id,
  action, state, created_at_ms, completed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, 'claimed', ?6, NULL);
