INSERT INTO legacy_notification_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'mark_postcondition',
  ?6,
  (
    SELECT COUNT(*)
    FROM notifications n
    WHERE n.organization_id = ?2
      AND n.recipient_user_id = ?3
      AND (?4 IS NULL OR n.id = ?4)
      AND n.read_at_ms = ?5
      AND n.last_operation_id = ?1
  )
)
