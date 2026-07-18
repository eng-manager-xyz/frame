INSERT INTO auth_delivery_ack_tombstones_v2(
  operation_id, delivery_id, lease_id, attempt,
  lease_expires_at_ms, acknowledged_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
