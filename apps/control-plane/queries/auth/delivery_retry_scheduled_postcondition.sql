SELECT CASE WHEN EXISTS (
  SELECT 1 FROM auth_delivery_outbox_v2 d
  WHERE d.delivery_id = ?1
    AND d.attempt = ?2
    AND d.lease_id IS NULL
    AND d.lease_expires_at_ms IS NULL
    AND d.next_attempt_at_ms = ?3
    AND d.last_operation_id = ?4
) THEN 1 ELSE 0 END AS present
