INSERT INTO legacy_membership_action_authority_subjects_v1(
  operation_id, user_id, generation_before, generation_after
)
SELECT
  ?1,
  affected.user_id,
  COALESCE(generation.generation, 0),
  COALESCE(generation.generation, 0) + 1
FROM (
  SELECT user_id FROM legacy_membership_action_previous_members_v1 WHERE operation_id = ?1
  UNION
  SELECT user_id FROM legacy_membership_action_final_members_v1 WHERE operation_id = ?1
) affected
LEFT JOIN legacy_membership_authority_generations_v1 generation
  ON generation.organization_id = ?2 AND generation.user_id = affected.user_id
ORDER BY affected.user_id
