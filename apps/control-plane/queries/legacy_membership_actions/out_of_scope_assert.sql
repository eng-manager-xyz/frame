INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'out_of_scope', 0,
  (SELECT COUNT(*) FROM space_members member
    WHERE member.last_operation_id = ?1 AND member.space_id <> ?2)
)
