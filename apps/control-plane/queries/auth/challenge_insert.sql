INSERT INTO auth_verification_challenges_v2(
  id, user_id, initiator_session_id, initiator_user_id, initiator_generation,
  provisioning_revision, identifier_key_version, identifier_digest,
  secret_key_version, secret_digest, purpose, channel,
  attempt_count, max_attempts, created_at_ms, expires_at_ms,
  consumed_at_ms, state, revision, last_operation_id
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
  ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
  ?17, ?18, 0, ?19
)
