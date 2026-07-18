SELECT user_id, legacy_user_id, legacy_member_id, mapped_member_id,
  role, state, revision
FROM legacy_membership_action_previous_members_v1
WHERE operation_id = ?1
ORDER BY user_id
LIMIT 100001
