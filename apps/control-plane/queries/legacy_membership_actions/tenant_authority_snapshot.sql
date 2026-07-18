SELECT
  actor.organization_preference_revision AS selection_revision,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  membership.role AS membership_role,
  membership.state AS membership_state,
  membership.revision AS membership_revision,
  membership.authority_version AS membership_authority_version,
  CASE
    WHEN organization.owner_id = actor.id THEN 'organization_owner'
    WHEN membership.role = 'admin' THEN 'organization_admin'
    ELSE 'active_organization_member'
  END AS actor_authority
FROM users actor
JOIN organizations organization
  ON organization.id = actor.active_organization_id
 AND organization.id = ?2
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
    OR membership.state = 'active'
  )
LIMIT 2
