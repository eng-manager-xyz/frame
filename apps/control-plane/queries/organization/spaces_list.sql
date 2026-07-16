SELECT id, organization_id, created_by_user_id, name, is_primary, is_public,
       settings_json, created_at_ms, updated_at_ms, deleted_at_ms, revision
FROM spaces
WHERE organization_id = ?1
  AND deleted_at_ms IS NULL
  AND (?2 IS NULL OR id > ?2)
ORDER BY id
LIMIT ?3
