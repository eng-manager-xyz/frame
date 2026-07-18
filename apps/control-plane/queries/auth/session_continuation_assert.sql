INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?5,
       CASE WHEN EXISTS (
         SELECT 1
         FROM auth_sessions_v2 s
         JOIN auth_identities_v2 i ON i.user_id = s.user_id
         JOIN users u ON u.id = s.user_id AND u.status = 'active'
         WHERE s.id = ?1
           AND s.user_id = ?2
           AND s.generation = ?3
           AND s.state = 'active'
           AND s.issued_at_ms <= ?4
           AND s.idle_expires_at_ms > ?4
           AND s.absolute_expires_at_ms > ?4
           AND s.session_version = i.session_version
       ) THEN 1 ELSE 0 END
