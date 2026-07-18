INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'folder_authority', 1,
  (
    SELECT COUNT(*)
    FROM users actor
    JOIN organizations organization
      ON organization.id = actor.active_organization_id
     AND organization.id = ?3
     AND organization.status = 'active'
     AND organization.tombstoned_at_ms IS NULL
    JOIN folders folder
      ON folder.id = ?4
     AND folder.organization_id = organization.id
     AND folder.deleted_at_ms IS NULL
    LEFT JOIN organization_members membership
      ON membership.organization_id = organization.id
     AND membership.user_id = actor.id
    LEFT JOIN space_members space_membership
      ON space_membership.space_id = folder.space_id
     AND space_membership.user_id = actor.id
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND organization.revision = ?5
      AND organization.authority_version = ?6
      AND folder.revision = ?7
      AND folder.tree_revision = ?8
      AND folder.legacy_organization_library_revision = ?9
      AND (
        (folder.legacy_scope_kind = 'personal' AND folder.created_by_user_id = actor.id)
        OR (
          folder.legacy_scope_kind = 'organization'
          AND (
            organization.owner_id = actor.id
            OR (membership.state = 'active' AND membership.role = 'admin')
          )
        )
        OR (
          folder.legacy_scope_kind = 'space'
          AND (
            organization.owner_id = actor.id
            OR (membership.state = 'active' AND membership.role = 'admin')
            OR folder.created_by_user_id = actor.id
            OR (space_membership.state = 'active' AND space_membership.role = 'manager')
          )
        )
      )
  )
)
