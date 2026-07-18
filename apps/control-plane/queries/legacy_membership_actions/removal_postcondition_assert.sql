INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation_postcondition', 0,
  (SELECT COUNT(*)
    FROM legacy_membership_action_previous_members_v1 previous
    WHERE previous.operation_id = ?1
      AND (
        EXISTS (
          SELECT 1 FROM space_members current
          WHERE current.space_id = ?2 AND current.user_id = previous.user_id
        )
        OR EXISTS (
          SELECT 1 FROM legacy_space_member_aliases_v1 alias
          WHERE alias.mapped_member_id = previous.mapped_member_id
            AND alias.removed_at_ms IS NULL
        )
      ))
)
