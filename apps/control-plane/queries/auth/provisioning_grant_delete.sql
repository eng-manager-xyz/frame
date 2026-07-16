DELETE FROM auth_identity_provisioning_grants_v2
WHERE id = ?1
  AND user_id = ?2
  AND identity_revision = ?3
  AND identifier_key_version = ?4
  AND identifier_digest = ?5
  AND expires_at_ms = ?6
  AND expires_at_ms > ?7
