SELECT operation_id, organization_id, actor_id, action, request_digest, state,
       result_json, effects_json
FROM legacy_organization_library_operations_v1
WHERE actor_id = ?1 AND action = ?2 AND idempotency_key_digest = ?3
LIMIT 2
