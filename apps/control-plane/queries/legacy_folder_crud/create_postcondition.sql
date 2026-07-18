INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation', 1,
  (
    SELECT COUNT(*) FROM folders
    WHERE id = ?2 AND legacy_folder_id = ?3 AND legacy_name = ?8
      AND name = CASE WHEN length(?8) = 0 THEN char(8291) ELSE ?8 END
      AND organization_id = ?4
      AND space_id IS ?5 AND parent_id IS ?6 AND created_by_user_id = ?7
      AND legacy_color = ?9 AND is_public = ?10
      AND settings_json = '{}' AND deleted_at_ms IS NULL
      AND revision = 0 AND depth = ?11 AND tree_revision = 0
      AND last_operation_id = ?1
      AND legacy_scope_kind = ?12 AND legacy_scope_id IS ?13
  )
)
