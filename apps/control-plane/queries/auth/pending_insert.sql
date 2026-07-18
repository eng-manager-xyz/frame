INSERT INTO auth_pending_verifications_v2(
  delivery_id, identifier_candidates_json,
  active_identifier_key_version, active_identifier_digest,
  secret_key_version, secret_digest, purpose, channel,
  initiator_session_id, initiator_user_id, initiator_generation,
  provisioning_user_id, provisioning_revision, max_attempts,
  created_at_ms, expires_at_ms, sealed_payload_hex,
  revision, last_operation_id
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
  ?9, ?10, ?11, ?12, ?13, ?14,
  ?15, ?16, ?17, 0, ?18
)
