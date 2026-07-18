SELECT COUNT(*) AS recent_count
FROM auth_api_keys
WHERE user_id = ?1
  AND created_at_ms > ?2 - 3600000
