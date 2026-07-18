INSERT INTO legacy_organization_library_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'member_authority', 1,
  (
    SELECT COUNT(*)
    FROM users actor
    JOIN organizations organization
      ON organization.id = actor.active_organization_id
     AND organization.id = ?3
     AND organization.status = 'active'
     AND organization.tombstoned_at_ms IS NULL
    JOIN legacy_invite_lifecycle_member_aliases_v1 alias
      ON alias.legacy_member_id = ?4
     AND alias.organization_id = organization.id
     AND alias.removed_at_ms IS NULL
    JOIN organization_members target
      ON target.organization_id = alias.organization_id
     AND target.user_id = alias.user_id
     AND target.state = 'active'
    LEFT JOIN organization_members actor_membership
      ON actor_membership.organization_id = organization.id
     AND actor_membership.user_id = actor.id
    WHERE actor.id = ?2
      AND actor.status = 'active'
      AND actor.deleted_at_ms IS NULL
      AND target.revision = ?5
      AND target.authority_version = ?6
      AND (
        organization.owner_id = actor.id
        OR (actor_membership.state = 'active' AND actor_membership.role = 'admin')
      )
  )
)
