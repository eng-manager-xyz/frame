INSERT INTO auth_sessions_v2(
  id, family_id, user_id, client_kind,
  token_key_version, token_digest, csrf_key_version, csrf_digest, browser_origin,
  issued_at_ms, rotated_at_ms, idle_expires_at_ms, absolute_expires_at_ms,
  session_version, generation, state, revoked_at_ms, revocation_reason,
  revision, last_operation_id
) VALUES (
  ?1, ?2, ?3, ?4,
  ?5, ?6, ?7, ?8, ?9,
  ?10, ?10, ?11, ?12,
  ?13, 0, 'active', NULL, NULL,
  0, ?14
)
