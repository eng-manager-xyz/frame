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
WHERE suppress = 0
  AND expires_at_ms > ?1
  AND attempt < 12
  AND next_attempt_at_ms <= ?1
  AND (lease_expires_at_ms IS NULL OR lease_expires_at_ms <= ?1)
ORDER BY next_attempt_at_ms, created_at_ms, delivery_id
LIMIT 1
