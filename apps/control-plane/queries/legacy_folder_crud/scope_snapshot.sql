SELECT
  ?3 AS scope_kind,
  ?4 AS scope_id,
  COALESCE(s.revision, -1) AS scope_revision,
  COALESCE(s.authority_version, -1) AS scope_authority_version,
  COALESCE(s.created_by_user_id, '') AS scope_creator_id,
  COALESCE(sm.role, '') AS actor_space_role,
  COALESCE(sm.revision, -1) AS actor_space_membership_revision
FROM organizations o
LEFT JOIN spaces s
  ON ?3 = 'space'
 AND s.id = ?4
 AND s.organization_id = o.id
 AND s.deleted_at_ms IS NULL
LEFT JOIN space_members sm
  ON sm.space_id = s.id
 AND sm.user_id = ?2
 AND sm.state = 'active'
WHERE o.id = ?1
  AND o.status = 'active'
  AND o.tombstoned_at_ms IS NULL
  AND (
    (?3 = 'personal' AND ?4 IS NULL)
    OR (?3 = 'organization' AND ?4 = o.id)
    OR (?3 = 'space' AND s.id IS NOT NULL)
  )
LIMIT 2
