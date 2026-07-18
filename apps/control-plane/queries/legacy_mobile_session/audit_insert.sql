INSERT INTO legacy_mobile_session_audit_events_v1(
  event_id, operation_id, actor_id, action, subject_digest, outcome, occurred_at_ms
) VALUES(?1, ?2, ?3, ?4, ?5, 'allow', ?6)
