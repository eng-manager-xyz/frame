UPDATE legacy_space_member_aliases_v1
SET removed_at_ms = ?2
WHERE removed_at_ms IS NULL
  AND mapped_member_id IN (
    SELECT mapped_member_id
    FROM legacy_membership_action_previous_members_v1
    WHERE operation_id = ?1
  )
