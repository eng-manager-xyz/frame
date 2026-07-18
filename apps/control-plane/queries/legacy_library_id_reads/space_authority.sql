SELECT space.id AS space_id
FROM legacy_library_space_aliases_v1 space_alias
JOIN spaces space
  ON space.id = space_alias.space_id
 AND space.organization_id = ?3
 AND space.deleted_at_ms IS NULL
LEFT JOIN space_members membership
  ON membership.space_id = space.id
 AND membership.user_id = ?1
 AND membership.state = 'active'
WHERE space_alias.legacy_space_id = ?2
  AND (
    space.created_by_user_id = ?1
    OR space.is_public = 1
    OR membership.user_id IS NOT NULL
  )
LIMIT 2;
