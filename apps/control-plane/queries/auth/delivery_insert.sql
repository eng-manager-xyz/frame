INSERT INTO auth_delivery_outbox_v2(
  delivery_id, sealed_payload_hex, suppress,
  created_at_ms, expires_at_ms, next_attempt_at_ms,
  attempt, lease_id, lease_expires_at_ms, initiator_session_id,
  revision, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?4, 0, NULL, NULL, ?6, 0, ?7)
