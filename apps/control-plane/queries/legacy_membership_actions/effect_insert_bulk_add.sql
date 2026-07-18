INSERT INTO legacy_membership_action_effects_v1(
  operation_id, organization_id, space_id,
  invalidates_organization_invites, invalidates_space_page,
  invalidates_space_members, bumps_authority_generation,
  authority_subject_count, revalidation_path, created_at_ms
)
SELECT ?1, ?2, ?3, 0, 1, 1,
  CASE WHEN COUNT(*) = 0 THEN 0 ELSE 1 END,
  COUNT(*), ?4, ?5
FROM legacy_membership_action_authority_subjects_v1
WHERE operation_id = ?1
