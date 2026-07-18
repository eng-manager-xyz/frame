INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'previous_bound', 0,
  CASE WHEN (
    SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1
    WHERE operation_id = ?1
  ) <= 100000 THEN 0 ELSE 1 END
)
