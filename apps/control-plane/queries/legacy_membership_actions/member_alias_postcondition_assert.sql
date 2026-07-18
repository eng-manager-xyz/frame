INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'alias_postcondition', 0,
  (
    SELECT COUNT(*)
    FROM legacy_membership_action_final_members_v1 final
    LEFT JOIN legacy_space_member_aliases_v1 alias
      ON alias.mapped_member_id = final.mapped_member_id
     AND alias.legacy_member_id = final.legacy_member_id
     AND alias.legacy_user_id = final.legacy_user_id
     AND alias.space_id = ?2
     AND alias.user_id = final.user_id
     AND alias.removed_at_ms IS NULL
    WHERE final.operation_id = ?1 AND alias.mapped_member_id IS NULL
  )
)
