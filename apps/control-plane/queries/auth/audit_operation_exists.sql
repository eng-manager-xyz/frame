SELECT EXISTS (
  SELECT 1
  FROM auth_audit_events_v2
  WHERE operation_id = ?1
    AND action = ?2
    AND outcome = ?3
    AND reason = ?4
) AS present
