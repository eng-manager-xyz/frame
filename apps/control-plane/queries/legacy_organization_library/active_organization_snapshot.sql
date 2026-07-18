SELECT
  organization.id AS organization_id,
  organization.owner_id,
  organization.name,
  organization.status,
  organization.settings_json,
  organization.legacy_icon_key,
  organization.legacy_shareable_link_icon_key,
  organization.legacy_workos_organization_id,
  organization.legacy_workos_connection_id,
  organization.legacy_allowed_email_restriction,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  organization.legacy_organization_library_revision,
  actor.organization_preference_revision,
  actor.legacy_invite_quota,
  actor.legacy_stripe_subscription_id,
  actor.legacy_stripe_subscription_status,
  actor.legacy_third_party_stripe_subscription_id,
  membership.role AS actor_role,
  membership.state AS actor_membership_state,
  membership.revision AS actor_membership_revision,
  membership.authority_version AS actor_membership_authority_version
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
