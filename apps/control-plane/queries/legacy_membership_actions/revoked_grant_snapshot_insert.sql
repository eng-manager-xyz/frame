INSERT INTO legacy_membership_action_revoked_grants_v1(
  operation_id, mutation_grant_id, session_id, user_id
)
SELECT ?1, grant_row.id, grant_row.session_id, grant_row.user_id
FROM auth_session_mutation_grants_v2 grant_row
JOIN legacy_membership_action_authority_subjects_v1 subject
  ON subject.operation_id = ?1 AND subject.user_id = grant_row.user_id
ORDER BY grant_row.id
