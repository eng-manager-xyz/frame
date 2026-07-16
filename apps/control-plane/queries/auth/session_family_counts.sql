SELECT
  (SELECT COUNT(*) FROM auth_sessions_v2 WHERE family_id = ?1 AND state = 'active') AS active_sessions,
  (SELECT COUNT(*) FROM auth_session_credentials_v2 WHERE family_id = ?1 AND state <> 'revoked') AS live_credentials
