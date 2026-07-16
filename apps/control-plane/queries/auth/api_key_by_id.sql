SELECT id,
       owner_id,
       tenant_id,
       key_version,
       key_digest,
       scopes_json,
       created_at_ms,
       expires_at_ms,
       revoked_at_ms,
       revision,
       0 AS candidate_order
FROM auth_api_keys_v2
WHERE id = ?1
LIMIT 1
