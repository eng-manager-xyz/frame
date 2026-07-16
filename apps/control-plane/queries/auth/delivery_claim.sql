UPDATE auth_delivery_outbox_v2
SET attempt = attempt + 1,
    lease_id = ?4,
    lease_expires_at_ms = MIN(?5, expires_at_ms),
    revision = revision + 1,
    last_operation_id = ?6
WHERE delivery_id = ?1
  AND revision = ?2
  AND attempt = ?3
  AND suppress = 0
  AND expires_at_ms > ?7
  AND attempt < 12
  AND next_attempt_at_ms <= ?7
  AND (lease_expires_at_ms IS NULL OR lease_expires_at_ms <= ?7)
