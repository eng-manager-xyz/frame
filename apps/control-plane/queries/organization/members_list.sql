SELECT organization_id, user_id, role, state, has_pro_seat,
       created_at_ms, updated_at_ms, revision, authority_version
FROM organization_members
WHERE organization_id = ?1
  AND (?2 IS NULL OR user_id > ?2)
ORDER BY user_id
LIMIT ?3
