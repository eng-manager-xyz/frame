INSERT INTO legacy_invite_lifecycle_audit_events_v1(
  operation_id, actor_id, organization_id, action, occurred_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5);
