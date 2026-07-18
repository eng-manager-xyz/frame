INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'authority_generation', COUNT(*), changes()
FROM legacy_membership_action_authority_subjects_v1
WHERE operation_id = ?1
