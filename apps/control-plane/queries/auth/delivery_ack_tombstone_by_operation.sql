SELECT operation_id,
       delivery_id,
       lease_id,
       attempt,
       lease_expires_at_ms,
       acknowledged_at_ms
FROM auth_delivery_ack_tombstones_v2
WHERE operation_id = ?1
LIMIT 1
