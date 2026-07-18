INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'mutation', ?2,
  (
    (SELECT COUNT(*) FROM legacy_folder_crud_delete_targets_v1 WHERE operation_id = ?1)
    + CASE WHEN
      (SELECT json_group_array(folder_id)
       FROM (
         SELECT folder_id FROM legacy_folder_crud_delete_targets_v1
         WHERE operation_id = ?1 ORDER BY folder_id
       )) = ?3
      AND
      (SELECT COUNT(*) FROM folders f
       JOIN legacy_folder_crud_delete_targets_v1 target ON target.folder_id = f.id
       WHERE target.operation_id = ?1) = 0
      AND (SELECT COUNT(*) FROM videos v
           JOIN legacy_folder_crud_delete_targets_v1 target ON target.folder_id = v.folder_id
           WHERE target.operation_id = ?1) = 0
      AND (SELECT COUNT(*) FROM space_videos sv
           JOIN legacy_folder_crud_delete_targets_v1 target ON target.folder_id = sv.folder_id
           WHERE target.operation_id = ?1) = 0
      AND (SELECT COUNT(*) FROM shared_videos shared
           JOIN legacy_folder_crud_delete_targets_v1 target ON target.folder_id = shared.folder_id
           WHERE target.operation_id = ?1 AND shared.revoked_at_ms IS NULL) = 0
    THEN 0 ELSE 1 END
  )
)
