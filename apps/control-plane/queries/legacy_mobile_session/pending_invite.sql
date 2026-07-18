SELECT EXISTS(
  SELECT 1
  FROM legacy_invite_lifecycle_invite_aliases_v1 invite_alias
  JOIN organization_invites invite
    ON invite.id = invite_alias.mapped_invite_id
   AND invite.status = 'pending'
  WHERE invite_alias.invited_email = ?1 COLLATE NOCASE
    AND invite_alias.decision = 'pending'
) AS pending_invite
