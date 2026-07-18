SELECT
  alias.legacy_member_id,
  alias.organization_id,
  alias.user_id AS target_user_id,
  target.role AS target_role,
  target.state AS target_state,
  target.has_pro_seat,
  target.revision AS target_revision,
  target.authority_version AS target_authority_version,
  organization.owner_id,
  actor_membership.role AS actor_role,
  actor_membership.state AS actor_state,
  actor_membership.revision AS actor_revision,
  actor_membership.authority_version AS actor_authority_version,
  target_user.email AS target_email,
  target_user.legacy_third_party_stripe_subscription_id,
  owner.legacy_invite_quota AS owner_invite_quota,
  owner.legacy_stripe_subscription_id AS owner_subscription_id,
  owner.legacy_stripe_subscription_status AS owner_subscription_status,
  actor.legacy_invite_quota AS actor_invite_quota,
  actor.legacy_stripe_subscription_id AS actor_subscription_id,
  actor.legacy_stripe_subscription_status AS actor_subscription_status
FROM users actor
JOIN organizations organization
  ON organization.id = actor.active_organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN legacy_invite_lifecycle_member_aliases_v1 alias
  ON alias.legacy_member_id = ?3
 AND alias.organization_id = organization.id
 AND alias.removed_at_ms IS NULL
JOIN organization_members target
  ON target.organization_id = alias.organization_id
 AND target.user_id = alias.user_id
 AND target.state = 'active'
JOIN users target_user ON target_user.id = target.user_id
JOIN users owner ON owner.id = organization.owner_id
LEFT JOIN organization_members actor_membership
  ON actor_membership.organization_id = organization.id
 AND actor_membership.user_id = actor.id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND (
    organization.owner_id = actor.id
    OR (actor_membership.state = 'active' AND actor_membership.role = 'admin')
  )
LIMIT 2
