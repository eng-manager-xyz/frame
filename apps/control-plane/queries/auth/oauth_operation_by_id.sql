SELECT operation_id,
       operation_kind,
       subject_id,
       result_code,
       result_timestamp_ms,
       request_fingerprint,
       committed_at_ms
FROM auth_oauth_operations_v2
WHERE operation_id = ?1
LIMIT 1
