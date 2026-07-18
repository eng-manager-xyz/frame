INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'aliases_complete',
  (SELECT COUNT(*) FROM space_members WHERE space_id = ?2),
  (SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1
    WHERE operation_id = ?1)
)
