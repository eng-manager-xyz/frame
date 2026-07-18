SELECT
  organization.id AS organization_id,
  organization.owner_id,
  organization.name,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  actor.organization_preference_revision,
  membership.role AS actor_role,
  membership.state AS actor_membership_state,
  membership.revision AS actor_membership_revision,
  membership.authority_version AS actor_membership_authority_version
FROM users actor
JOIN organizations organization
  ON organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = actor.id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND (
    organization.owner_id = actor.id
    OR (membership.state = 'active' AND membership.role = 'admin')
  )
LIMIT 2
