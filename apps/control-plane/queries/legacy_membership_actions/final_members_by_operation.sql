SELECT user_id, legacy_user_id, legacy_member_id, mapped_member_id, role, ordinal
FROM legacy_membership_action_final_members_v1
WHERE operation_id = ?1
ORDER BY ordinal, user_id
LIMIT 502
