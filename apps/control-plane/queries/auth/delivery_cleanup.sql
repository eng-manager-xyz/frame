DELETE FROM auth_delivery_outbox_v2
WHERE suppress = 1
   OR expires_at_ms <= ?1
   OR (
     attempt >= 12
     AND (lease_id IS NULL OR lease_expires_at_ms <= ?1)
   )
