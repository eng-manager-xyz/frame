SELECT
  u.preferences_json,
  u.notification_preferences_revision
FROM users u
WHERE u.id = ?1
  AND u.status = 'active'
  AND u.deleted_at_ms IS NULL
LIMIT 2
