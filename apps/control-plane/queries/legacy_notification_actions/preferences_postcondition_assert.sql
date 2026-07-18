INSERT INTO legacy_notification_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'preferences_postcondition',
  1,
  (
    SELECT COUNT(*)
    FROM users u
    WHERE u.id = ?2
      AND u.status = 'active'
      AND u.deleted_at_ms IS NULL
      AND u.notification_preferences_revision = ?3 + 1
      AND u.notification_preferences_last_operation_id = ?1
      AND u.preferences_json = ?4
      AND json_extract(u.preferences_json, '$.notifications') = json(?5)
  )
)
