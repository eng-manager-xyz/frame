INSERT INTO folders(
  id, legacy_folder_id, legacy_name, organization_id, space_id, parent_id,
  created_by_user_id, name, legacy_color, is_public, settings_json,
  created_at_ms, updated_at_ms, deleted_at_ms, revision,
  depth, tree_revision, last_operation_id, legacy_scope_kind, legacy_scope_id
)
VALUES (
  ?1, ?2, ?7, ?3, ?4, ?5,
  ?6, CASE WHEN length(?7) = 0 THEN char(8291) ELSE ?7 END, ?8, ?9, '{}',
  ?10, ?10, NULL, 0,
  ?11, 0, ?12, ?13, ?14
)
