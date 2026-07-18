SELECT
  f.id,
  f.legacy_folder_id,
  f.organization_id,
  f.space_id,
  f.legacy_scope_kind AS scope_kind,
  f.legacy_scope_id AS scope_id,
  f.parent_id,
  f.created_by_user_id,
  f.name AS storage_name,
  f.legacy_name,
  COALESCE(f.legacy_name, f.name) AS name,
  f.legacy_color AS color,
  f.is_public,
  f.settings_json,
  f.revision,
  f.tree_revision,
  f.depth,
  COALESCE(s.revision, -1) AS scope_revision,
  COALESCE(s.authority_version, -1) AS scope_authority_version,
  COALESCE(s.created_by_user_id, '') AS scope_creator_id,
  COALESCE(sm.role, '') AS actor_space_role,
  COALESCE(sm.revision, -1) AS actor_space_membership_revision
FROM folders f
LEFT JOIN spaces s
  ON f.legacy_scope_kind = 'space'
 AND s.id = f.space_id
 AND s.organization_id = f.organization_id
 AND s.deleted_at_ms IS NULL
LEFT JOIN space_members sm
  ON sm.space_id = s.id
 AND sm.user_id = ?3
 AND sm.state = 'active'
WHERE f.id = ?1
  AND f.organization_id = ?2
  AND f.deleted_at_ms IS NULL
  AND (f.legacy_scope_kind <> 'space' OR s.id IS NOT NULL)
LIMIT 2
