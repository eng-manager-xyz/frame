DELETE FROM auth_session_mutation_grants_v2
WHERE session_id IN (SELECT id FROM auth_sessions_v2 WHERE user_id = ?1)
