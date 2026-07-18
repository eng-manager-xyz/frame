INSERT INTO auth_api_keys(
  id, user_id, key_digest, name, scopes_json, created_at_ms,
  expires_at_ms, last_used_at_ms, revoked_at_ms, legacy_source
) VALUES(?1, ?2, ?3, 'Cap mobile', '["frame:read","frame:write"]', ?4,
         NULL, NULL, NULL, 'mobile')
