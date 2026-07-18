INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'grant_revocation_postcondition', 0, COUNT(*)
FROM auth_session_mutation_grants_v2 grant_row
JOIN legacy_membership_action_authority_subjects_v1 subject
  ON subject.operation_id = ?1 AND subject.user_id = grant_row.user_id
