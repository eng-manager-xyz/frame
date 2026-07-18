SELECT token_digest, created_at_ms, expires_at_ms
FROM legacy_mobile_session_challenges_v1
WHERE identifier_digest = ?1
LIMIT 1
