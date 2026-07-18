DELETE FROM notifications
WHERE id IN (
  SELECT notification_id
  FROM legacy_collaboration_notification_targets_v1
  WHERE operation_id = ?1
);
