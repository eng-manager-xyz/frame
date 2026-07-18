SELECT
  u.organization_preference_revision AS selection_revision,
  o.owner_id,
  o.revision AS organization_revision,
  o.authority_version AS organization_authority_version,
  m.role AS membership_role,
  m.revision AS membership_revision,
  m.authority_version AS membership_authority_version,
  owner_membership.has_pro_seat AS owner_has_pro_seat,
  owner_membership.revision AS owner_membership_revision,
  owner_membership.authority_version AS owner_membership_authority_version
FROM users u
JOIN organizations o
  ON o.id = u.active_organization_id
 AND o.status = 'active'
 AND o.tombstoned_at_ms IS NULL
JOIN organization_members m
  ON m.organization_id = o.id
 AND m.user_id = u.id
 AND m.state = 'active'
JOIN organization_members owner_membership
  ON owner_membership.organization_id = o.id
 AND owner_membership.user_id = o.owner_id
 AND owner_membership.role = 'owner'
 AND owner_membership.state = 'active'
WHERE u.id = ?1
  AND u.status = 'active'
  AND u.deleted_at_ms IS NULL
  AND u.active_organization_id = ?2
LIMIT 2
