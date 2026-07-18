UPDATE notifications
SET read_at_ms = ?5,
    revision = revision + 1,
    last_operation_id = ?1
WHERE organization_id = ?2
  AND recipient_user_id = ?3
  AND (?4 IS NULL OR id = ?4)
