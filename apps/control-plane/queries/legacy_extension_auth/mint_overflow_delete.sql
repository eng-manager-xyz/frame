DELETE FROM auth_api_keys
WHERE id = ?1
  AND user_id = ?2
  AND legacy_source = 'extension'
  AND (
    SELECT COUNT(*)
    FROM auth_api_keys recent
    WHERE recent.user_id = ?2
      AND recent.created_at_ms > ?3 - 3600000
  ) > 10
