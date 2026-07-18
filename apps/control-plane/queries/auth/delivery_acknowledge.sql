DELETE FROM auth_delivery_outbox_v2
WHERE delivery_id = ?1
  AND revision = ?2
  AND attempt = ?3
  AND lease_id = ?4
  AND lease_expires_at_ms = ?5
  AND lease_expires_at_ms > ?6
