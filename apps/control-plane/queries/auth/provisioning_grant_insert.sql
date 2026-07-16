INSERT INTO auth_identity_provisioning_grants_v2(
  id, user_id, identity_revision,
  identifier_key_version, identifier_digest,
  expires_at_ms, created_at_ms, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
