DELETE FROM auth_verification_challenges_v2
WHERE expires_at_ms <= ?1
