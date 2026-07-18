SELECT u.preferences_json
FROM users u
WHERE u.id = ?1
LIMIT 1
