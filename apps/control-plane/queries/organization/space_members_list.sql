SELECT sm.space_id, s.organization_id, sm.user_id, sm.role, sm.state,
       sm.created_at_ms, sm.updated_at_ms, sm.revision
FROM space_members sm
JOIN spaces s ON s.id = sm.space_id
WHERE s.organization_id = ?1
  AND s.id = ?2
  AND s.deleted_at_ms IS NULL
  AND (?3 IS NULL OR sm.user_id > ?3)
ORDER BY sm.user_id
LIMIT ?4
