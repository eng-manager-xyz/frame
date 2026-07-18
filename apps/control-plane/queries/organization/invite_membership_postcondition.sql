INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM organization_members m
         JOIN organization_invites invitation
           ON invitation.organization_id = m.organization_id
          AND invitation.accepted_by_user_id = m.user_id
         WHERE invitation.id = ?2 AND invitation.organization_id = ?3
           AND m.user_id = ?4 AND m.state = 'active'
           AND m.role = invitation.role AND m.role <> 'owner'
           AND m.revision = 0 AND m.last_operation_id = ?5
       ) THEN 1 ELSE 0 END
