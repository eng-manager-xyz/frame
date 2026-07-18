INSERT INTO legacy_collaboration_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'tenant_authority', 1, COUNT(*)
FROM users user
JOIN organizations organization ON organization.id = ?3
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id AND membership.user_id = user.id
LEFT JOIN legacy_collaboration_user_aliases_v1 alias
  ON alias.mapped_user_id = user.id
WHERE user.id = ?2
  AND user.status = 'active' AND user.deleted_at_ms IS NULL
  AND user.active_organization_id = organization.id
  AND user.organization_preference_revision = ?4
  AND user.updated_at_ms = ?5
  AND organization.status = 'active' AND organization.tombstoned_at_ms IS NULL
  AND organization.revision = ?6
  AND organization.authority_version = ?7
  AND membership.role IS ?8
  AND membership.state IS ?9
  AND membership.revision IS ?10
  AND membership.authority_version IS ?11
  AND user.display_name IS ?12
  AND COALESCE(
    alias.legacy_user_id,
    (SELECT MIN(member_alias.legacy_user_id)
      FROM legacy_space_member_aliases_v1 member_alias
      WHERE member_alias.user_id = user.id),
    substr(replace(user.id, '-', ''), 1, 15)
  ) = ?13
  AND alias.image_url IS ?14
  AND (
    organization.owner_id = user.id
    OR (membership.state = 'active' AND membership.role IN ('owner', 'admin', 'member', 'viewer'))
  );
