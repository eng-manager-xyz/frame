INSERT INTO legacy_membership_authority_generations_v1(
  organization_id, user_id, generation, updated_at_ms, last_operation_id
)
SELECT ?2, subject.user_id, subject.generation_after, ?3, ?1
FROM legacy_membership_action_authority_subjects_v1 subject
WHERE subject.operation_id = ?1
ON CONFLICT(organization_id, user_id) DO UPDATE SET
  generation = excluded.generation,
  updated_at_ms = excluded.updated_at_ms,
  last_operation_id = excluded.last_operation_id
