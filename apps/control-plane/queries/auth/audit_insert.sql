INSERT INTO auth_audit_events_v2(
  id, correlation_id, user_id, session_id, client_kind,
  action, outcome, reason, occurred_at_ms, operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
