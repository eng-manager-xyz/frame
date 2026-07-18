DELETE FROM space_members
WHERE space_id = ?2
  AND user_id IN (
    SELECT user_id FROM legacy_membership_action_previous_members_v1
    WHERE operation_id = ?1
  )
