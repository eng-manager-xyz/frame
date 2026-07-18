INSERT INTO legacy_mobile_session_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'stripe_effect_pending', 1, COUNT(*)
FROM legacy_mobile_session_stripe_effects_v1
WHERE operation_id = ?1 AND user_id = ?2
  AND normalized_email_digest = ?3 AND state = 'pending'
