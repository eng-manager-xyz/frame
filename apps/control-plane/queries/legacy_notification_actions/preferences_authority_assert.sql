INSERT INTO legacy_notification_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'preferences_authority',
  1,
  (
    SELECT COUNT(*)
    FROM users u
    WHERE u.id = ?2
      AND u.status = 'active'
      AND u.deleted_at_ms IS NULL
      AND u.notification_preferences_revision = ?3
      AND u.preferences_json IS ?4
  )
)
