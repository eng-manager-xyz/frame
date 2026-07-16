INSERT INTO space_members(
  space_id, user_id, role, created_at_ms, updated_at_ms, state, revision, last_operation_id
)
SELECT ?1, ?2, ?3, ?5, ?5, ?4, 0, ?6
WHERE EXISTS (
  SELECT 1 FROM spaces s JOIN organization_members m
    ON m.organization_id = s.organization_id AND m.user_id = ?2
  WHERE s.id = ?1 AND s.organization_id = ?7 AND s.deleted_at_ms IS NULL AND m.state = 'active'
)
ON CONFLICT(space_id, user_id) DO UPDATE SET
  role = excluded.role,
  state = excluded.state,
  updated_at_ms = excluded.updated_at_ms,
  revision = space_members.revision + 1,
  last_operation_id = excluded.last_operation_id
WHERE space_members.revision = ?8
