INSERT INTO auth_oauth_flows_v2(
  id, provider, purpose,
  initiator_session_id, initiator_user_id, initiator_generation,
  state_key_version, state_digest,
  pkce_key_version, pkce_digest,
  redirect_key_version, redirect_digest,
  audience_key_version, audience_digest,
  created_at_ms, expires_at_ms, consumed_at_ms, revoked, revision,
  last_operation_id
) VALUES (
  ?1, ?2, ?3,
  ?4, ?5, ?6,
  ?7, ?8,
  ?9, ?10,
  ?11, ?12,
  ?13, ?14,
  ?15, ?16, NULL, 0, 0,
  ?17
)
