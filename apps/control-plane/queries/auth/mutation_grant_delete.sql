DELETE FROM auth_session_mutation_grants_v2
WHERE id = ?1
  AND session_id = ?2
  AND user_id = ?3
  AND generation = ?4
  AND token_key_version = ?5
  AND token_digest = ?6
  AND EXISTS (
    SELECT 1
    FROM auth_sessions_v2 s
    JOIN auth_identities_v2 i ON i.user_id = s.user_id
    JOIN users u ON u.id = s.user_id AND u.status = 'active'
    WHERE s.id = ?2
      AND s.user_id = ?3
      AND s.generation = ?4
      AND s.token_key_version = ?5
      AND s.token_digest = ?6
      AND s.state = 'active'
      AND s.issued_at_ms <= ?7
      AND s.idle_expires_at_ms > ?7
      AND s.absolute_expires_at_ms > ?7
      AND s.session_version = i.session_version
  )
