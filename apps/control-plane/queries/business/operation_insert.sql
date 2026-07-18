INSERT INTO business_repository_operations_v1(
  operation_id, organization_id, principal_kind, principal_subject,
  idempotency_key, action, subject_id, request_fingerprint, result_code,
  resulting_revision, committed_at_ms
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
