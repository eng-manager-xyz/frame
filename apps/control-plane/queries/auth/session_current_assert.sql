INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?9,
       CASE WHEN EXISTS (
         SELECT 1
         FROM auth_sessions_v2 s
         JOIN auth_session_credentials_v2 c
           ON c.session_id = s.id
          AND c.key_version = ?5
          AND c.digest = ?6
          AND c.state = ?7
          AND c.revision = ?8
         JOIN auth_identities_v2 i ON i.user_id = s.user_id
         JOIN users u ON u.id = s.user_id AND u.status = 'active'
         WHERE s.id = ?1
           AND s.revision = ?2
           AND s.generation = ?3
           AND s.state = ?4
           AND s.session_version = i.session_version
           AND s.issued_at_ms <= ?10
           AND s.idle_expires_at_ms > ?10
           AND s.absolute_expires_at_ms > ?10
       ) THEN 1 ELSE 0 END
