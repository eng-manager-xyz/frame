DELETE FROM auth_principal_issuance_grants_v2
WHERE id = ?1
  AND user_id = ?2
  AND identity_revision = ?3
  AND expires_at_ms = ?4
  AND expires_at_ms > ?5
