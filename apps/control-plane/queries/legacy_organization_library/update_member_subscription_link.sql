UPDATE users
SET legacy_third_party_stripe_subscription_id = ?2, updated_at_ms = ?3
WHERE id = ?1 AND status = 'active' AND deleted_at_ms IS NULL
