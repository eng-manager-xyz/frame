SELECT delivery_id,
       sealed_payload_hex,
       created_at_ms,
       expires_at_ms,
       next_attempt_at_ms,
       attempt,
       lease_id,
       lease_expires_at_ms,
       revision
FROM auth_delivery_outbox_v2
WHERE last_operation_id = ?1
LIMIT 2
