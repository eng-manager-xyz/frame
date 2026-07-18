INSERT INTO organization_repository_operations_v1(
  operation_id, organization_id, idempotency_key, operation_kind,
  subject_id, request_fingerprint, result_code, resulting_revision,
  authority_version, committed_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
