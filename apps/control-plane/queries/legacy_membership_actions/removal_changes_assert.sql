INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'members_deleted', COUNT(*), changes()
FROM legacy_membership_action_previous_members_v1
WHERE operation_id = ?1
