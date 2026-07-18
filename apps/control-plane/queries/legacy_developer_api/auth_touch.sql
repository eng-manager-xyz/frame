UPDATE legacy_developer_api_keys_v1
SET last_used_at_ms = ?2
WHERE key_digest = ?1
  AND (last_used_at_ms IS NULL OR last_used_at_ms < ?2 - 300000)
