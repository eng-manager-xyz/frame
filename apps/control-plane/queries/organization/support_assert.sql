INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM organization_support_authorities_v1 s
         JOIN organizations o
           ON o.id = s.organization_id
          AND o.status IN ('active', 'tombstoned')
         JOIN users u ON u.id = s.support_actor_id AND u.status = 'active'
         JOIN auth_identities_v2 i ON i.user_id = u.id
         WHERE s.support_actor_id = ?2
           AND s.organization_id = ?3
           AND s.ticket_digest = ?4
           AND s.revoked_at_ms IS NULL
           AND s.expires_at_ms > CAST(strftime('%s', 'now') AS INTEGER) * 1000
           AND i.identity_revision = ?5
           AND i.session_version = ?6
       ) THEN 1 ELSE 0 END
