INSERT INTO space_members(
  space_id, user_id, role, created_at_ms, updated_at_ms,
  state, revision, last_operation_id
)
SELECT ?2, final.user_id, final.role, ?3, ?3, 'active', 0, ?1
FROM legacy_membership_action_final_members_v1 final
WHERE final.operation_id = ?1
  AND NOT EXISTS (
    SELECT 1 FROM legacy_membership_action_previous_members_v1 previous
    WHERE previous.operation_id = ?1 AND previous.user_id = final.user_id
  )
ORDER BY final.ordinal
