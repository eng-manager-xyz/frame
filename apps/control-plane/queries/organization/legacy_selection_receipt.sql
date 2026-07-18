SELECT operation_id,
       organization_id,
       idempotency_key,
       operation_kind,
       subject_id,
       request_fingerprint,
       result_code,
       resulting_revision,
       authority_version,
       committed_at_ms
FROM organization_repository_operations_v1
WHERE operation_id = ?1
