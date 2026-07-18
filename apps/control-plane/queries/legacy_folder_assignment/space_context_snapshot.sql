SELECT
  s.id,
  s.revision,
  s.authority_version,
  COALESCE(sm.role, '') AS actor_space_role,
  COALESCE(sm.revision, -1) AS actor_space_membership_revision
FROM spaces s
LEFT JOIN space_members sm
  ON sm.space_id = s.id
 AND sm.user_id = ?3
 AND sm.state = 'active'
WHERE s.id = ?1
  AND s.organization_id = ?2
  AND s.deleted_at_ms IS NULL
LIMIT 2
