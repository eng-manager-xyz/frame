SELECT user_id, generation_before, generation_after
FROM legacy_membership_action_authority_subjects_v1
WHERE operation_id = ?1
ORDER BY user_id
LIMIT 100502
