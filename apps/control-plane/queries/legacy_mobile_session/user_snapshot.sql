SELECT u.id AS mapped_user_id,
       a.legacy_user_id,
       u.display_name,
       EXISTS(
         SELECT 1 FROM identity_accounts account WHERE account.user_id = u.id
       ) AS has_linked_account,
       EXISTS(
         SELECT 1
         FROM legacy_invite_lifecycle_invite_aliases_v1 invite_alias
         JOIN organization_invites invite
           ON invite.id = invite_alias.mapped_invite_id
          AND invite.status = 'pending'
         JOIN organization_members member
           ON member.organization_id = invite_alias.organization_id
          AND member.user_id = u.id
          AND member.state = 'active'
         WHERE invite_alias.invited_email = ?1 COLLATE NOCASE
           AND invite_alias.decision = 'pending'
       ) AS has_pending_provisioned_invite
FROM users u
LEFT JOIN legacy_collaboration_user_aliases_v1 a ON a.mapped_user_id = u.id
WHERE u.email = ?1 COLLATE NOCASE
LIMIT 1
