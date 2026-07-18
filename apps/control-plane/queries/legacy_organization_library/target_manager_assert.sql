INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'target_manager_authority', 1,
  (
    SELECT COUNT(*)
    FROM users actor
    JOIN organizations organization
      ON organization.id = ?3
     AND organization.status = 'active'
     AND organization.tombstoned_at_ms IS NULL
    LEFT JOIN organization_members membership
      ON membership.organization_id = organization.id
     AND membership.user_id = actor.id
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND actor.organization_preference_revision = ?4
      AND organization.revision = ?5
      AND organization.authority_version = ?6
      AND (
        organization.owner_id = actor.id
        OR (membership.state = 'active' AND membership.role = 'admin')
      )
  )
)
