INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM organization_invites invitation
         JOIN organizations o ON o.id = invitation.organization_id AND o.status = 'active'
         JOIN users u ON u.id = ?5 AND u.status = 'active'
         JOIN auth_identities_v2 i ON i.user_id = u.id
         JOIN auth_identifier_digests_v2 identifier
           ON identifier.user_id = u.id
          AND identifier.key_version = invitation.invited_email_key_version
          AND identifier.digest = invitation.invited_email_digest
         WHERE invitation.id = ?2
           AND invitation.organization_id = ?3
           AND invitation.status = 'pending'
           AND invitation.revision = ?4
           AND invitation.expires_at_ms > CAST(strftime('%s', 'now') AS INTEGER) * 1000
           AND invitation.token_digest = ?6
           AND invitation.role <> 'owner'
           AND i.identity_revision = ?7
           AND i.session_version = ?8
           AND NOT EXISTS (
             SELECT 1 FROM organization_members m
             WHERE m.organization_id = invitation.organization_id AND m.user_id = ?5
           )
       ) THEN 1 ELSE 0 END
