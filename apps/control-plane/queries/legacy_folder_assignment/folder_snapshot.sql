SELECT
  f.id,
  f.space_id,
  f.parent_id,
  f.created_by_user_id,
  f.revision,
  f.tree_revision,
  COALESCE(s.revision, -1) AS space_revision,
  COALESCE(s.authority_version, -1) AS space_authority_version,
  COALESCE(sm.role, '') AS actor_space_role,
  COALESCE(sm.revision, -1) AS actor_space_membership_revision
FROM folders f
LEFT JOIN spaces s
  ON s.id = f.space_id
 AND s.organization_id = f.organization_id
 AND s.deleted_at_ms IS NULL
LEFT JOIN space_members sm
  ON sm.space_id = s.id
 AND sm.user_id = ?3
 AND sm.state = 'active'
WHERE f.id = ?1
  AND f.organization_id = ?2
  AND f.deleted_at_ms IS NULL
  AND (f.space_id IS NULL OR s.id IS NOT NULL)
LIMIT 2
