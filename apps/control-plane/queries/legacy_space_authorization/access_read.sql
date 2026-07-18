SELECT
  space_alias.legacy_space_id AS legacy_space_id,
  organization_alias.legacy_organization_id AS legacy_organization_id,
  organization.owner_id AS organization_owner_user_id,
  space.created_by_user_id AS created_by_user_id,
  owner_alias.legacy_user_id AS legacy_organization_owner_id,
  creator_alias.legacy_user_id AS legacy_created_by_id,
  (
    SELECT MIN(owner_member_alias.legacy_user_id)
    FROM legacy_space_member_aliases_v1 owner_member_alias
    WHERE owner_member_alias.user_id = organization.owner_id
  ) AS membership_organization_owner_id,
  (
    SELECT MIN(creator_member_alias.legacy_user_id)
    FROM legacy_space_member_aliases_v1 creator_member_alias
    WHERE creator_member_alias.user_id = space.created_by_user_id
  ) AS membership_created_by_id,
  CASE WHEN organization.owner_id = actor.id THEN 1 ELSE 0 END
    AS actor_is_organization_owner,
  CASE WHEN space.created_by_user_id = actor.id THEN 1 ELSE 0 END
    AS actor_is_space_creator,
  organization_membership.role AS organization_member_role,
  CASE space_membership.role
    WHEN 'manager' THEN 'admin'
    WHEN 'viewer' THEN 'member'
    ELSE space_membership.role
  END AS space_member_role
FROM users actor
JOIN organizations organization
  ON organization.id = actor.active_organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.organization_id = organization.id
 AND organization_alias.legacy_organization_id = ?3
JOIN legacy_library_space_aliases_v1 space_alias
  ON space_alias.legacy_space_id = ?4
JOIN spaces space
  ON space.id = space_alias.space_id
 AND space.organization_id = organization.id
 AND space.deleted_at_ms IS NULL
LEFT JOIN organization_members organization_membership
  ON organization_membership.organization_id = organization.id
 AND organization_membership.user_id = actor.id
 AND organization_membership.state = 'active'
LEFT JOIN space_members space_membership
  ON space_membership.space_id = space.id
 AND space_membership.user_id = actor.id
 AND space_membership.state = 'active'
LEFT JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = organization.owner_id
LEFT JOIN legacy_collaboration_user_aliases_v1 creator_alias
  ON creator_alias.mapped_user_id = space.created_by_user_id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND (
    organization.owner_id = actor.id
    OR organization_membership.user_id IS NOT NULL
  )
LIMIT 2;
