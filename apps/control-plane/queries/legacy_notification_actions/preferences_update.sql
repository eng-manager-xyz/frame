UPDATE users
SET preferences_json = ?5,
    notification_preferences_revision = notification_preferences_revision + 1,
    notification_preferences_last_operation_id = ?1
WHERE id = ?2
  AND status = 'active'
  AND deleted_at_ms IS NULL
  AND notification_preferences_revision = ?3
  AND preferences_json IS ?4
