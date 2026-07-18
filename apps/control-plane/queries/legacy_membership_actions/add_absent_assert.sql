INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'add_absent', 0,
  (SELECT COUNT(*) FROM space_members member
    WHERE member.space_id = ?2 AND member.user_id = ?3)
)
