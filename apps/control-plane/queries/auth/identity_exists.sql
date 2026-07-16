SELECT 1 AS present
FROM auth_identities_v2
WHERE user_id = ?1
LIMIT 1
