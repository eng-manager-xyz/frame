INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation', 1,
  (
    SELECT COUNT(*) FROM folders
    WHERE id = ?2 AND organization_id = ?3 AND deleted_at_ms IS NULL
      AND COALESCE(legacy_name, name) = ?4
      AND name = ?5
      AND legacy_name IS ?6
      AND legacy_color = ?7 AND is_public = ?8
      AND settings_json = CASE
        WHEN ?9 = 1 THEN json_patch(?10, json_object('publicPage', json(?11)))
        ELSE ?10
      END
      AND parent_id IS ?12
      AND revision = ?13 AND tree_revision = ?14 AND depth = ?15
      AND (?16 = 0 OR last_operation_id = ?1)
  )
)
