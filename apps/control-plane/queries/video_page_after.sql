SELECT id,
       title,
       state,
       privacy,
       revision,
       created_at_ms,
       updated_at_ms
FROM videos
WHERE organization_id = ?1
  AND deleted_at_ms IS NULL
  AND (created_at_ms, id) < (?2, ?3)
ORDER BY created_at_ms DESC, id DESC
LIMIT ?4
