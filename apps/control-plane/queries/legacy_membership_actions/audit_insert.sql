INSERT INTO legacy_membership_action_audit_events_v1(
  id, operation_id, organization_id, actor_id, action,
  principal_subject_digest, mutation_subject_digest, outcome, occurred_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'allow', ?8)
