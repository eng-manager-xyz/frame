INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'browser_grant', 1,
  (
    SELECT COUNT(*)
    FROM auth_session_mutation_grants_v2 grant_row
    JOIN auth_sessions_v2 session_row
      ON session_row.id = grant_row.session_id
     AND session_row.user_id = grant_row.user_id
    JOIN auth_identities_v2 identity_row
      ON identity_row.user_id = grant_row.user_id
    JOIN users actor
      ON actor.id = grant_row.user_id
     AND actor.status = 'active'
     AND actor.deleted_at_ms IS NULL
    WHERE grant_row.id = ?2
      AND grant_row.session_id = ?3
      AND grant_row.user_id = ?4
      AND session_row.client_kind = 'browser'
      AND session_row.state = 'active'
      AND session_row.generation = grant_row.generation
      AND session_row.token_key_version = grant_row.token_key_version
      AND session_row.token_digest = grant_row.token_digest
      AND session_row.session_version = identity_row.session_version
      AND session_row.idle_expires_at_ms > ?5
      AND session_row.absolute_expires_at_ms > ?5
  )
)
