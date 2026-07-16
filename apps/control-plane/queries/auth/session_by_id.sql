SELECT s.id,
       s.family_id,
       s.user_id,
       s.client_kind,
       s.token_key_version,
       s.token_digest,
       s.csrf_key_version,
       s.csrf_digest,
       s.browser_origin,
       s.issued_at_ms,
       s.rotated_at_ms,
       s.idle_expires_at_ms,
       s.absolute_expires_at_ms,
       s.session_version,
       s.generation,
       s.state,
       s.revoked_at_ms,
       s.revocation_reason,
       s.revision AS session_revision,
       i.session_version AS current_session_version,
       u.status AS user_status
FROM auth_sessions_v2 s
JOIN auth_identities_v2 i ON i.user_id = s.user_id
JOIN users u ON u.id = s.user_id
WHERE s.id = ?1
LIMIT 1
