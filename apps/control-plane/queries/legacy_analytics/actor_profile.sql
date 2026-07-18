SELECT id, COALESCE(NULLIF(display_name, ''), NULLIF(email, ''), 'Someone') AS display_name
FROM users
WHERE id = ?1 AND deleted_at_ms IS NULL
LIMIT 1
