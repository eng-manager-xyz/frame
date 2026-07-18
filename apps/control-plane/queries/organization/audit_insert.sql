INSERT INTO organization_audit_events_v1(
  id, operation_id, organization_id, actor_id, action, subject_kind,
  subject_digest, outcome, denial_code, occurred_at_ms, metadata_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}')
