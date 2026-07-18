INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'member_targets', 1,
  (SELECT CASE WHEN COUNT(*) >= ?2 THEN 1 ELSE 0 END
    FROM legacy_membership_action_previous_members_v1 WHERE operation_id = ?1)
)
