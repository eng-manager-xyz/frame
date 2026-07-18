INSERT INTO legacy_notification_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'other_actor',
  0,
  (
    SELECT COUNT(*)
    FROM users u
    WHERE u.notification_preferences_last_operation_id = ?1
      AND u.id <> ?2
  )
)
