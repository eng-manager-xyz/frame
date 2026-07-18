SELECT
  space.id AS space_id,
  space.organization_id,
  space.created_by_user_id,
  space.name,
  space.is_public,
  space.settings_json,
  space.legacy_icon_key,
  space.revision AS space_revision,
  space.authority_version AS space_authority_version,
  space.legacy_organization_library_revision,
  organization.owner_id,
  organization.settings_json AS organization_settings_json,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  owner.legacy_stripe_subscription_id AS owner_subscription_id,
  owner.legacy_stripe_subscription_status AS owner_subscription_status,
  owner.legacy_third_party_stripe_subscription_id AS owner_third_party_subscription_id,
  membership.role AS actor_role,
  membership.state AS actor_membership_state,
  membership.revision AS actor_membership_revision,
  membership.authority_version AS actor_membership_authority_version,
  space_membership.role AS actor_space_role,
  space_membership.state AS actor_space_state,
  space_membership.revision AS actor_space_revision
FROM users actor
JOIN spaces space
  ON space.id = ?3 AND space.deleted_at_ms IS NULL
JOIN organizations organization
  ON organization.id = space.organization_id
 AND organization.id = actor.active_organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN users owner ON owner.id = organization.owner_id
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
      membership.state = 'active' AND (
        membership.role = 'admin'
        OR space.created_by_user_id = actor.id
        OR (space_membership.state = 'active' AND space_membership.role = 'manager')
      )
    )
  )
LIMIT 2
