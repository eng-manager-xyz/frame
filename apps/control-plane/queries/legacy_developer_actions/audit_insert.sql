INSERT INTO legacy_developer_action_audit_events_v1(
  id, operation_id, actor_id, action, subject_digest, outcome, occurred_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, 'allow', ?6)
