SELECT id, organization_id, space_id, parent_id, created_by_user_id, name,
       is_public, settings_json, depth, created_at_ms, updated_at_ms,
       deleted_at_ms, revision, tree_revision
FROM folders
WHERE organization_id = ?1
  AND space_id = ?2
  AND deleted_at_ms IS NULL
  AND (?3 IS NULL OR id > ?3)
ORDER BY id
LIMIT ?4
