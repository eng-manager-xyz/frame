SELECT COUNT(*) AS total
FROM videos video
JOIN organizations organization
  ON organization.id = video.organization_id
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
LEFT JOIN folders folder
  ON folder.id = video.folder_id
WHERE video.owner_id = ?1
  AND video.organization_id = ?2
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
  AND (
    (?3 IS NULL AND video.folder_id IS NULL)
    OR (?3 IS NOT NULL AND folder.legacy_folder_id = ?3)
  );
