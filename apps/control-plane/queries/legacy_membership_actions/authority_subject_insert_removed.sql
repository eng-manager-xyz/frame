INSERT INTO legacy_membership_action_authority_subjects_v1(
  operation_id, user_id, generation_before, generation_after
)
SELECT ?1, previous.user_id, COALESCE(generation.generation, 0),
  COALESCE(generation.generation, 0) + 1
FROM legacy_membership_action_previous_members_v1 previous
LEFT JOIN legacy_membership_authority_generations_v1 generation
  ON generation.organization_id = ?2 AND generation.user_id = previous.user_id
WHERE previous.operation_id = ?1
ORDER BY previous.user_id
