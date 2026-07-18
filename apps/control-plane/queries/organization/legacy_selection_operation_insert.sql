INSERT INTO organization_repository_operations_v1(
  operation_id, organization_id, idempotency_key, operation_kind,
  subject_id, request_fingerprint, result_code, resulting_revision,
  authority_version, committed_at_ms
)
SELECT ?1,
       ?2,
       ?3,
       'active_organization_set',
       ?4,
       ?5,
       'applied',
       u.organization_preference_revision,
       o.authority_version,
       ?6
FROM users u
JOIN organizations o ON o.id = ?2
WHERE u.id = ?4
  AND u.active_organization_id = ?2
  AND u.organization_last_operation_id = ?1
