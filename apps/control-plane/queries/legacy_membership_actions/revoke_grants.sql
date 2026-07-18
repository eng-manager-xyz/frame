DELETE FROM auth_session_mutation_grants_v2
WHERE id IN (
  SELECT revoked.mutation_grant_id
  FROM legacy_membership_action_revoked_grants_v1 revoked
  WHERE revoked.operation_id = ?1
)
