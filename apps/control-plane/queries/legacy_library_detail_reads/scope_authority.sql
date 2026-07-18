SELECT authority.scope_kind AS scope_kind
FROM (
  SELECT 'organization' AS scope_kind
  FROM organizations organization
  WHERE ?4 = ?3
    AND organization.id = ?2
    AND organization.status = 'active'
    AND organization.tombstoned_at_ms IS NULL

  UNION ALL

  SELECT 'space' AS scope_kind
  FROM legacy_library_space_aliases_v1 space_alias
  JOIN spaces space
    ON space.id = space_alias.space_id
   AND space.organization_id = ?2
   AND space.deleted_at_ms IS NULL
  LEFT JOIN space_members membership
    ON membership.space_id = space.id
   AND membership.user_id = ?1
   AND membership.state = 'active'
  WHERE ?4 <> ?3
    AND space_alias.legacy_space_id = ?4
    AND (
      space.created_by_user_id = ?1
      OR space.is_public = 1
      OR membership.user_id IS NOT NULL
    )
) authority
LIMIT 2;
