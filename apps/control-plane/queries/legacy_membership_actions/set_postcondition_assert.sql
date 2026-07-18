INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation_postcondition', 1,
  CASE WHEN
    (SELECT COUNT(*) FROM space_members WHERE space_id = ?2)
      = (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1
          WHERE operation_id = ?1)
    AND NOT EXISTS (
      SELECT final.user_id
      FROM legacy_membership_action_final_members_v1 final
      WHERE final.operation_id = ?1
      EXCEPT
      SELECT member.user_id
      FROM space_members member
      JOIN legacy_membership_action_final_members_v1 final
        ON final.operation_id = ?1
       AND final.user_id = member.user_id
       AND final.role = member.role
      WHERE member.space_id = ?2
        AND member.state = 'active'
        AND member.last_operation_id = ?1
    )
    AND EXISTS (
      SELECT 1 FROM space_members creator
      WHERE creator.space_id = ?2 AND creator.user_id = ?3
        AND creator.role = 'manager' AND creator.state = 'active'
        AND creator.last_operation_id = ?1
    )
  THEN 1 ELSE 0 END
)
