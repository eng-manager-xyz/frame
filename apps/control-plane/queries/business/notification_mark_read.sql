UPDATE notifications
SET read_at_ms=COALESCE(read_at_ms, ?4), last_operation_id=?5
WHERE id=?1 AND organization_id=?2 AND recipient_user_id=?3
  AND created_at_ms<=?4 AND (read_at_ms IS NULL OR read_at_ms=?4)
