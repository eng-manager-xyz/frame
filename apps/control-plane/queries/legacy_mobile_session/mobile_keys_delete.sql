DELETE FROM auth_api_keys
WHERE user_id = ?1 AND legacy_source = 'mobile'
