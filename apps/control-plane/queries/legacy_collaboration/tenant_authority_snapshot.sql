SELECT
  user.organization_preference_revision AS selection_revision,
  user.updated_at_ms AS user_updated_at_ms,
  organization.revision AS organization_revision,
  organization.authority_version AS organization_authority_version,
  membership.role AS membership_role,
  membership.state AS membership_state,
  membership.revision AS membership_revision,
  membership.authority_version AS membership_authority_version,
  user.display_name AS author_name,
  COALESCE(
    alias.legacy_user_id,
    (SELECT MIN(member_alias.legacy_user_id)
      FROM legacy_space_member_aliases_v1 member_alias
      WHERE member_alias.user_id = user.id),
    substr(replace(user.id, '-', ''), 1, 15)
  ) AS legacy_author_id,
  alias.image_url AS author_image,
  CASE
    WHEN alias.legacy_user_id IS NOT NULL THEN alias.provenance
    WHEN EXISTS (
      SELECT 1 FROM legacy_space_member_aliases_v1 member_alias
      WHERE member_alias.user_id = user.id
    ) THEN 'membership_backfill'
    ELSE 'native_generated'
  END AS alias_provenance
FROM users user
JOIN organizations organization ON organization.id = ?2
LEFT JOIN organization_members membership
  ON membership.organization_id = organization.id AND membership.user_id = user.id
LEFT JOIN legacy_collaboration_user_aliases_v1 alias
  ON alias.mapped_user_id = user.id
WHERE user.id = ?1
  AND user.status = 'active' AND user.deleted_at_ms IS NULL
  AND user.active_organization_id = organization.id
  AND organization.status = 'active' AND organization.tombstoned_at_ms IS NULL
  AND (
    organization.owner_id = user.id
    OR (membership.state = 'active' AND membership.role IN ('owner', 'admin', 'member', 'viewer'))
  )
LIMIT 2;
