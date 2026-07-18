INSERT INTO auth_api_keys(
  id, user_id, key_digest, name, scopes_json, created_at_ms,
  expires_at_ms, last_used_at_ms, revoked_at_ms, legacy_source
)
SELECT ?1, u.id, ?3, 'Cap Chrome extension', '["frame:read","frame:write"]', ?4,
       NULL, NULL, NULL, 'extension'
FROM users u
WHERE u.id = ?2
  AND u.status = 'active'
  AND u.deleted_at_ms IS NULL
