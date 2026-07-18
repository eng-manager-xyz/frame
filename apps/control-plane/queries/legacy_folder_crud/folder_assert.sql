INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, ?2, 1,
  (
    SELECT COUNT(*)
    FROM folders f
    LEFT JOIN spaces s
      ON f.legacy_scope_kind = 'space'
     AND s.id = f.space_id
     AND s.organization_id = f.organization_id
     AND s.deleted_at_ms IS NULL
    LEFT JOIN space_members sm
      ON sm.space_id = s.id
     AND sm.user_id = ?5
     AND sm.state = 'active'
    WHERE f.id = ?3
      AND f.organization_id = ?4
      AND f.deleted_at_ms IS NULL
      AND f.space_id IS ?6
      AND f.legacy_scope_kind = ?7
      AND f.legacy_scope_id IS ?8
      AND f.parent_id IS ?9
      AND f.created_by_user_id = ?10
      AND f.name = ?11
      AND f.legacy_name IS ?12
      AND COALESCE(f.legacy_name, f.name) = ?13
      AND f.legacy_color = ?14
      AND f.is_public = ?15
      AND f.settings_json = ?16
      AND f.revision = ?17
      AND f.tree_revision = ?18
      AND f.depth = ?19
      AND COALESCE(s.revision, -1) = ?20
      AND COALESCE(s.authority_version, -1) = ?21
      AND COALESCE(s.created_by_user_id, '') = ?22
      AND COALESCE(sm.role, '') = ?23
      AND COALESCE(sm.revision, -1) = ?24
  )
)
