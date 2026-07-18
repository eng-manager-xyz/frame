SELECT
  actor.organization_preference_revision AS selection_revision,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  membership.role AS membership_role,
  membership.state AS membership_state,
  membership.revision AS membership_revision,
  membership.authority_version AS membership_authority_version,
  space.id AS space_id,
  space.created_by_user_id AS creator_id,
  space.revision AS space_revision,
  space.authority_version AS space_authority_version,
  space_membership.role AS space_membership_role,
  space_membership.state AS space_membership_state,
  space_membership.revision AS space_membership_revision,
  CASE
    WHEN organization.owner_id = actor.id THEN 'organization_owner'
    WHEN membership.role = 'admin' THEN 'organization_admin'
    WHEN space.created_by_user_id = actor.id THEN 'space_creator'
    ELSE 'space_manager'
  END AS actor_authority
FROM users actor
JOIN organizations organization
  ON organization.id = actor.active_organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN spaces space
  ON space.id = ?3
 AND space.organization_id = organization.id
 AND space.deleted_at_ms IS NULL
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = actor.id
LEFT JOIN space_members space_membership
  ON space_membership.space_id = space.id
 AND space_membership.user_id = actor.id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND (
    organization.owner_id = actor.id
    OR (
      membership.state = 'active'
      AND (
        membership.role = 'admin'
        OR space.created_by_user_id = actor.id
        OR (
          space_membership.state = 'active'
          AND space_membership.role = 'manager'
        )
      )
    )
  )
LIMIT 2
