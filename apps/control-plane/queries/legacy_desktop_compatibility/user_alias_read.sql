SELECT legacy_user_id
FROM legacy_collaboration_user_aliases_v1
WHERE mapped_user_id = ?1
LIMIT 2;
