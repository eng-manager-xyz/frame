INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'space_authority', 1,
  (
    SELECT COUNT(*)
    FROM users actor
    JOIN organizations organization
      ON organization.id = actor.active_organization_id
     AND organization.id = ?3
     AND organization.status = 'active'
     AND organization.tombstoned_at_ms IS NULL
    JOIN spaces space
      ON space.id = ?4
     AND space.organization_id = organization.id
     AND space.deleted_at_ms IS NULL
    LEFT JOIN organization_members membership
      ON membership.organization_id = organization.id
     AND membership.user_id = actor.id
    LEFT JOIN space_members space_membership
      ON space_membership.space_id = space.id
     AND space_membership.user_id = actor.id
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND organization.revision = ?5
      AND organization.authority_version = ?6
      AND space.revision = ?7
      AND space.authority_version = ?8
      AND space.legacy_organization_library_revision = ?9
      AND (
        organization.owner_id = actor.id
        OR (
          membership.state = 'active' AND (
            membership.role = 'admin'
            OR space.created_by_user_id = actor.id
            OR (space_membership.state = 'active' AND space_membership.role = 'manager')
          )
        )
      )
  )
)
