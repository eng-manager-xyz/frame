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
    LEFT JOIN organization_members membership
      ON membership.organization_id = organization.id
     AND membership.user_id = actor.id
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND actor.organization_preference_revision = ?4
      AND membership.role IS ?7
      AND membership.state IS ?8
      AND membership.revision IS ?9
      AND membership.authority_version IS ?10
      AND CASE
        WHEN organization.owner_id = actor.id THEN 'organization_owner'
        WHEN membership.state = 'active' AND membership.role = 'admin'
          THEN 'organization_admin'
        ELSE 'denied'
      END = ?11
  )
)
