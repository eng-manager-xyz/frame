INSERT INTO legacy_collaboration_audit_events_v1(
  id, operation_id, organization_id, actor_id, action,
  request_digest, outcome, occurred_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'allow', ?7);
