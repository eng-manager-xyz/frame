INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'aliases_inserted', COUNT(*), changes()
FROM legacy_membership_action_final_members_v1 final
WHERE final.operation_id = ?1
  AND NOT EXISTS (
    SELECT 1 FROM legacy_membership_action_previous_members_v1 previous
    WHERE previous.operation_id = ?1 AND previous.user_id = final.user_id
  )
