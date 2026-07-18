SELECT folder.id AS folder_id
FROM folders folder
LEFT JOIN legacy_library_space_aliases_v1 space_alias
  ON space_alias.legacy_space_id = ?4
LEFT JOIN spaces space
  ON space.id = space_alias.space_id
 AND space.organization_id = ?2
 AND space.deleted_at_ms IS NULL
LEFT JOIN space_members space_membership
  ON space_membership.space_id = space.id
 AND space_membership.user_id = ?1
 AND space_membership.state = 'active'
WHERE folder.legacy_folder_id = ?3
  AND folder.organization_id = ?2
  AND folder.deleted_at_ms IS NULL
  AND (
    (
      ?4 = ?5
      AND folder.legacy_scope_kind = 'organization'
      AND folder.legacy_scope_id = ?2
    )
    OR (
      ?4 <> ?5
      AND folder.legacy_scope_kind = 'space'
      AND folder.legacy_scope_id = space.id
      AND (
        space.created_by_user_id = ?1
        OR space.is_public = 1
        OR space_membership.user_id IS NOT NULL
      )
    )
  )
LIMIT 2;
