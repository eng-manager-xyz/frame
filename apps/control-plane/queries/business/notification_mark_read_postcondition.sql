INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM notifications
  WHERE id=?2 AND organization_id=?3 AND recipient_user_id=?4
    AND read_at_ms=?5 AND last_operation_id=?6
) THEN 1 ELSE 0 END
