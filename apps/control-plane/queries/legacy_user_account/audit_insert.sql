INSERT INTO legacy_user_account_audit_events_v1(
  id, operation_id, actor_id, action, principal_subject_digest,
  subject_digest, outcome, occurred_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'allow', ?7);
