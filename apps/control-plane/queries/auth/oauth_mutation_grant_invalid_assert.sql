INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?8,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_session_mutation_grants_v2 grant_row
         JOIN auth_sessions_v2 session
           ON session.id = grant_row.session_id
          AND session.user_id = grant_row.user_id
         JOIN auth_identities_v2 identity ON identity.user_id = grant_row.user_id
         JOIN users user_row ON user_row.id = grant_row.user_id AND user_row.status = 'active'
         WHERE grant_row.id = ?1
           AND grant_row.session_id = ?2
           AND grant_row.user_id = ?3
           AND grant_row.generation = ?4
           AND grant_row.token_key_version = ?5
           AND grant_row.token_digest = ?6
           AND session.generation = ?4
           AND session.token_key_version = ?5
           AND session.token_digest = ?6
           AND session.state = 'active'
           AND session.issued_at_ms <= ?7
           AND session.idle_expires_at_ms > ?7
           AND session.absolute_expires_at_ms > ?7
           AND session.session_version = identity.session_version
       ) THEN 1 ELSE 0 END
