INSERT INTO legacy_mobile_session_stripe_effects_v1(
  effect_id, operation_id, user_id, normalized_email_digest,
  state, attempt, lease_id, lease_expires_at_ms, provider_receipt_digest,
  last_error_class, created_at_ms, updated_at_ms
) VALUES(?1, ?2, ?3, ?4, 'pending', 0, NULL, NULL, NULL, NULL, ?5, ?5)
