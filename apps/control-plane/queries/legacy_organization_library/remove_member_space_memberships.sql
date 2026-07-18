UPDATE space_members
SET state = 'removed', revision = revision + 1, updated_at_ms = ?3,
    last_operation_id = ?4
WHERE user_id = ?1 AND state = 'active'
  AND space_id IN (
    SELECT id FROM spaces WHERE organization_id = ?2 AND deleted_at_ms IS NULL
  )
