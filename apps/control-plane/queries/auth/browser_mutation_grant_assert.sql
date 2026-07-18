INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'mutation_grant',
  1,
  (
    SELECT COUNT(*)
    FROM auth_session_mutation_grants_v2 g
    JOIN auth_sessions_v2 s ON s.id = g.session_id AND s.user_id = g.user_id
    JOIN auth_identities_v2 i ON i.user_id = g.user_id
    JOIN users u ON u.id = g.user_id AND u.status = 'active'
      AND u.deleted_at_ms IS NULL
    WHERE g.id = ?2
      AND g.session_id = ?3
      AND g.user_id = ?4
      AND s.state = 'active'
      AND s.generation = g.generation
      AND s.token_key_version = g.token_key_version
      AND s.token_digest = g.token_digest
      AND s.session_version = i.session_version
      AND s.idle_expires_at_ms > ?5
      AND s.absolute_expires_at_ms > ?5
  )
)
