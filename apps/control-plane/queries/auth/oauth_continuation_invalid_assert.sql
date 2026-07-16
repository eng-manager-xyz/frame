INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?5,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_sessions_v2 session
         JOIN auth_identities_v2 identity ON identity.user_id = session.user_id
         JOIN users user_row ON user_row.id = session.user_id AND user_row.status = 'active'
         WHERE session.id = ?1
           AND session.user_id = ?2
           AND session.generation = ?3
           AND session.state = 'active'
           AND session.issued_at_ms <= ?4
           AND session.idle_expires_at_ms > ?4
           AND session.absolute_expires_at_ms > ?4
           AND session.session_version = identity.session_version
       ) THEN 1 ELSE 0 END
