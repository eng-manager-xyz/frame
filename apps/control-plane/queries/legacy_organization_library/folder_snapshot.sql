SELECT
  folder.id AS folder_id,
  folder.organization_id,
  folder.space_id,
  folder.created_by_user_id,
  folder.settings_json,
  folder.revision AS folder_revision,
  folder.tree_revision,
  folder.legacy_organization_library_revision,
  organization.owner_id,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  owner.legacy_stripe_subscription_id AS owner_subscription_id,
  owner.legacy_stripe_subscription_status AS owner_subscription_status,
  owner.legacy_third_party_stripe_subscription_id AS owner_third_party_subscription_id,
  membership.role AS actor_role,
  membership.state AS actor_membership_state,
  space_membership.role AS actor_space_role,
  space_membership.state AS actor_space_state
FROM users actor
JOIN folders folder
  ON folder.id = ?3 AND folder.deleted_at_ms IS NULL
JOIN organizations organization
  ON organization.id = folder.organization_id
 AND organization.id = actor.active_organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN users owner ON owner.id = organization.owner_id
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id
 AND membership.user_id = actor.id
LEFT JOIN space_members space_membership
  ON space_membership.space_id = folder.space_id
 AND space_membership.user_id = actor.id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND (
    (folder.legacy_scope_kind = 'personal' AND folder.created_by_user_id = actor.id)
    OR (
      folder.legacy_scope_kind = 'organization'
      AND (
        organization.owner_id = actor.id
        OR (membership.state = 'active' AND membership.role = 'admin')
      )
    )
    OR (
      folder.legacy_scope_kind = 'space'
      AND (
        organization.owner_id = actor.id
        OR (membership.state = 'active' AND membership.role = 'admin')
        OR folder.created_by_user_id = actor.id
        OR (space_membership.state = 'active' AND space_membership.role = 'manager')
      )
    )
  )
LIMIT 2
