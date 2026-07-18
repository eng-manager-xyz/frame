INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'action_authority', 1,
  (
    SELECT COUNT(*)
    FROM users actor
    JOIN organizations organization
      ON organization.id = actor.active_organization_id
     AND organization.id = ?3
     AND organization.status = 'active'
     AND organization.tombstoned_at_ms IS NULL
     AND organization.revision = ?5
     AND organization.authority_version = ?6
    JOIN spaces space
      ON space.id = ?11
     AND space.organization_id = organization.id
     AND space.deleted_at_ms IS NULL
     AND space.created_by_user_id = ?12
     AND space.revision = ?13
     AND space.authority_version = ?14
    LEFT JOIN organization_members membership
      ON membership.organization_id = organization.id
     AND membership.user_id = actor.id
    LEFT JOIN space_members space_membership
      ON space_membership.space_id = space.id
     AND space_membership.user_id = actor.id
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND actor.organization_preference_revision = ?4
      AND membership.role IS ?7
      AND membership.state IS ?8
      AND membership.revision IS ?9
      AND membership.authority_version IS ?10
      AND space_membership.role IS ?15
      AND space_membership.state IS ?16
      AND space_membership.revision IS ?17
      AND CASE
        WHEN organization.owner_id = actor.id THEN 'organization_owner'
        WHEN membership.state = 'active' AND membership.role = 'admin'
          THEN 'organization_admin'
        WHEN membership.state = 'active' AND space.created_by_user_id = actor.id
          THEN 'space_creator'
        WHEN membership.state = 'active'
          AND space_membership.state = 'active'
          AND space_membership.role = 'manager'
          THEN 'space_manager'
        ELSE 'denied'
      END = ?18
  )
)
