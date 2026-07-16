SELECT COUNT(*) AS bucket_count
FROM auth_session_credentials_v2
WHERE session_id = ?1 AND state <> 'revoked'
