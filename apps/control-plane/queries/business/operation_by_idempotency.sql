SELECT operation_id, organization_id, principal_kind, principal_subject,
       idempotency_key, action, subject_id, request_fingerprint, result_code,
       resulting_revision, committed_at_ms
FROM business_repository_operations_v1
WHERE organization_id = ?1 AND principal_kind = ?2
  AND principal_subject = ?3 AND idempotency_key = ?4
LIMIT 2
