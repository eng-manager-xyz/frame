SELECT u.id AS mapped_user_id, a.legacy_user_id
FROM users u
LEFT JOIN legacy_collaboration_user_aliases_v1 a ON a.mapped_user_id = u.id
WHERE u.id = ?1
LIMIT 1
