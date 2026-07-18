SELECT
  (SELECT COUNT(*) FROM auth_sessions_v2 WHERE user_id = ?1 AND state = 'active') AS active_sessions,
  (SELECT COUNT(*) FROM auth_session_credentials_v2 c JOIN auth_sessions_v2 s ON s.id = c.session_id WHERE s.user_id = ?1 AND c.state <> 'revoked') AS live_credentials
