SELECT u.id AS mapped_user_id, a.legacy_user_id
FROM auth_api_keys key
JOIN users u ON u.id = key.user_id
LEFT JOIN legacy_collaboration_user_aliases_v1 a ON a.mapped_user_id = u.id
WHERE key.key_digest = ?1
  AND key.revoked_at_ms IS NULL
  AND (key.expires_at_ms IS NULL OR key.expires_at_ms > ?2)
LIMIT 1
