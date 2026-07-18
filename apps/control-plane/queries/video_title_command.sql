SELECT command_type,
       request_digest,
       response_status,
       response_json,
       expires_at_ms
FROM command_idempotency
WHERE organization_id = ?1
  AND idempotency_key = ?2
LIMIT 1
