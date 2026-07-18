INSERT INTO legacy_space_member_aliases_v1(
  mapped_member_id, legacy_member_id, legacy_user_id,
  space_id, user_id, created_at_ms
)
SELECT final.mapped_member_id, final.legacy_member_id, final.legacy_user_id,
  ?2, final.user_id, ?3
FROM legacy_membership_action_final_members_v1 final
WHERE final.operation_id = ?1
ORDER BY final.ordinal
