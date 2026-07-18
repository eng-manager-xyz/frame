SELECT id,
       user_id,
       identity_revision,
       identifier_key_version,
       identifier_digest,
       expires_at_ms
FROM auth_identity_provisioning_grants_v2
WHERE id = ?1
LIMIT 1
