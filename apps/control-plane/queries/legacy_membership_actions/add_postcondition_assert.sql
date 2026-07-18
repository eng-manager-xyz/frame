INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation_postcondition', 1,
  (
    SELECT COUNT(*)
    FROM space_members member
    JOIN legacy_membership_action_final_members_v1 final
      ON final.operation_id = ?1
     AND final.user_id = member.user_id
     AND final.role = member.role
    WHERE member.space_id = ?2
      AND member.state = 'active'
      AND member.last_operation_id = ?1
      AND (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1
            WHERE operation_id = ?1) = 1
  )
)
