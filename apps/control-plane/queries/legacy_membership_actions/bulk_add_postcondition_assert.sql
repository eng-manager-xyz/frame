INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation_postcondition', 1,
  CASE WHEN
    (SELECT COUNT(*) FROM space_members WHERE space_id = ?2)
      = (SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1
          WHERE operation_id = ?1)
        + (SELECT COUNT(*) FROM legacy_membership_action_final_members_v1 final
            WHERE final.operation_id = ?1
              AND NOT EXISTS (
                SELECT 1 FROM legacy_membership_action_previous_members_v1 previous
                WHERE previous.operation_id = ?1 AND previous.user_id = final.user_id
              ))
    AND NOT EXISTS (
      SELECT 1
      FROM legacy_membership_action_previous_members_v1 previous
      LEFT JOIN space_members current
        ON current.space_id = ?2
       AND current.user_id = previous.user_id
       AND current.role = previous.role
       AND current.state = previous.state
       AND current.revision = previous.revision
      WHERE previous.operation_id = ?1 AND current.user_id IS NULL
    )
    AND NOT EXISTS (
      SELECT 1
      FROM legacy_membership_action_final_members_v1 final
      LEFT JOIN space_members current
        ON current.space_id = ?2
       AND current.user_id = final.user_id
      WHERE final.operation_id = ?1
        AND current.user_id IS NULL
    )
  THEN 1 ELSE 0 END
)
