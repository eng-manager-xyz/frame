SELECT g.id,
       g.session_id,
       g.user_id,
       g.generation,
       g.token_key_version,
       g.token_digest,
       s.revision AS session_revision,
       s.state AS session_state,
       s.idle_expires_at_ms,
       s.absolute_expires_at_ms,
       s.session_version,
       s.generation AS current_generation,
       s.token_key_version AS current_token_key_version,
       s.token_digest AS current_token_digest,
       i.session_version AS current_session_version,
       u.status AS user_status
FROM auth_session_mutation_grants_v2 g
JOIN auth_sessions_v2 s ON s.id = g.session_id AND s.user_id = g.user_id
JOIN auth_identities_v2 i ON i.user_id = g.user_id
JOIN users u ON u.id = g.user_id
WHERE g.id = ?1
LIMIT 1
