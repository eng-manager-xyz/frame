DELETE FROM legacy_mobile_session_challenges_v1
WHERE identifier_digest = ?1 AND token_digest = ?2
