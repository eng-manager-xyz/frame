INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT
  ?1, 'authority_generation_postcondition', 0,
  COUNT(*)
FROM legacy_membership_action_authority_subjects_v1 subject
LEFT JOIN legacy_membership_authority_generations_v1 generation
  ON generation.organization_id = ?2
 AND generation.user_id = subject.user_id
 AND generation.generation = subject.generation_after
 AND generation.last_operation_id = ?1
WHERE subject.operation_id = ?1 AND generation.user_id IS NULL
