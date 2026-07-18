INSERT INTO auth_delivery_provider_handoffs_v1(
  delivery_id, payload_hex, payload_sha256, state, provider_attempt,
  provider_lease_id, provider_lease_expires_at_ms, next_attempt_at_ms,
  provider_receipt_digest, last_error_class, created_at_ms, updated_at_ms
) VALUES(?1, ?2, ?3, 'pending', 0, NULL, NULL, 0, NULL, NULL, ?4, ?4)
