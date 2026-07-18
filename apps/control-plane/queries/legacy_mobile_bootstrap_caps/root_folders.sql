SELECT
  folder.legacy_folder_id AS legacy_folder_id,
  COALESCE(folder.legacy_name, folder.name) AS name,
  folder.legacy_color AS color,
  parent.legacy_folder_id AS legacy_parent_id,
  (
    SELECT COUNT(*)
    FROM videos video
    WHERE video.folder_id = folder.id
      AND video.owner_id = ?1
      AND video.organization_id = ?2
      AND video.deleted_at_ms IS NULL
      AND video.state <> 'deleted'
  ) AS video_count
FROM folders folder
LEFT JOIN folders parent
  ON parent.id = folder.parent_id
WHERE folder.organization_id = ?2
  AND folder.created_by_user_id = ?1
  AND folder.parent_id IS NULL
  AND folder.space_id IS NULL
  AND folder.deleted_at_ms IS NULL;
