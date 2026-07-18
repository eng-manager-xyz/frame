UPDATE auth_delivery_outbox_v2
SET next_attempt_at_ms = ?7,
    lease_id = NULL,
    lease_expires_at_ms = NULL,
    revision = revision + 1,
    last_operation_id = ?8
WHERE delivery_id = ?1
  AND revision = ?2
  AND attempt = ?3
  AND lease_id = ?4
  AND lease_expires_at_ms = ?5
  AND lease_expires_at_ms > ?6
  AND attempt < 12
  AND ?7 < expires_at_ms
