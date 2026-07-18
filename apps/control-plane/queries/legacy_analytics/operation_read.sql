SELECT operation_id, request_digest, state
FROM legacy_analytics_provider_operations_v1
WHERE source_operation_id = ?1 AND principal_digest = ?2
  AND execution_key_digest = ?3
LIMIT 1
