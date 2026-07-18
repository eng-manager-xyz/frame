SELECT
  u.organization_preference_revision AS selection_revision,
  o.revision AS organization_revision,
  o.authority_version AS organization_authority_version,
  m.role AS membership_role,
  m.revision AS membership_revision,
  m.authority_version AS membership_authority_version
FROM users u
JOIN organizations o
  ON o.id = u.active_organization_id
 AND o.status = 'active'
JOIN organization_members m
  ON m.organization_id = o.id
 AND m.user_id = u.id
 AND m.state = 'active'
WHERE u.id = ?1
  AND u.status = 'active'
  AND u.deleted_at_ms IS NULL
  AND u.active_organization_id = ?2
LIMIT 2
