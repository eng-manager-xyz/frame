INSERT INTO business_audit_events_v1(
  id, operation_id, organization_id, principal_kind, principal_subject_digest,
  action, subject_digest, outcome, occurred_at_ms
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
