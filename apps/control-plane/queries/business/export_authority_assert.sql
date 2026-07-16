INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN ?3 = 'user' AND EXISTS (
         SELECT 1
         FROM organizations organization
         JOIN organization_members membership
           ON membership.organization_id = organization.id
          AND membership.user_id = ?4
          AND membership.state = 'active'
          AND membership.role = 'owner'
         JOIN users user ON user.id = membership.user_id AND user.status = 'active'
         JOIN auth_identities_v2 identity ON identity.user_id = user.id
         WHERE organization.id = ?2 AND organization.status = 'active'
           AND identity.identity_revision = ?5
           AND identity.session_version = ?6
       ) THEN 1 ELSE 0 END
