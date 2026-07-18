UPDATE folders
SET name = CASE
      WHEN ?2 = 0 THEN name
      WHEN length(?3) = 0 THEN char(8291)
      ELSE ?3
    END,
    legacy_name = CASE WHEN ?2 = 1 THEN ?3 ELSE legacy_name END,
    legacy_color = CASE WHEN ?4 = 1 THEN ?5 ELSE legacy_color END,
    is_public = CASE WHEN ?6 = 1 THEN ?7 ELSE is_public END,
    settings_json = CASE
      WHEN ?8 = 1 THEN json_patch(
        COALESCE(settings_json, '{}'),
        json_object('publicPage', json(?9))
      )
      ELSE settings_json
    END,
    parent_id = CASE
      WHEN ?10 = 'absent' THEN parent_id
      WHEN ?10 = 'root' THEN NULL
      ELSE ?11
    END,
    depth = CASE
      WHEN ?10 = 'absent' THEN depth
      WHEN ?10 = 'root' THEN 0
      ELSE ?12
    END,
    revision = revision + ?13,
    tree_revision = tree_revision + CASE WHEN ?10 = 'absent' THEN 0 ELSE 1 END,
    updated_at_ms = CASE WHEN ?13 = 1 THEN ?14 ELSE updated_at_ms END,
    last_operation_id = CASE WHEN ?13 = 1 THEN ?15 ELSE last_operation_id END
WHERE id = ?1
  AND organization_id = ?16
  AND deleted_at_ms IS NULL
  AND revision = ?17
  AND tree_revision = ?18
