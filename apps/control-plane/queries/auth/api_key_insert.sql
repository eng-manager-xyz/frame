INSERT INTO auth_api_keys_v2(
  id, owner_id, tenant_id, key_version, key_digest,
  scopes_json, created_at_ms, expires_at_ms, revoked_at_ms,
  revision, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 0, ?9)
