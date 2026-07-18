SELECT
  video_alias.legacy_video_id AS legacy_video_id,
  owner_alias.legacy_user_id AS legacy_owner_id,
  video.title AS video_name,
  video.created_at_ms AS created_at_ms,
  video.legacy_metadata_json AS metadata_json,
  video.legacy_is_screenshot AS is_screenshot,
  (
    SELECT COUNT(DISTINCT comment.legacy_comment_id)
    FROM legacy_collaboration_comments_v1 comment
    WHERE comment.legacy_video_id = video_alias.legacy_video_id
      AND comment.comment_kind = 'text'
  ) AS total_comments,
  (
    SELECT COUNT(DISTINCT reaction.legacy_comment_id)
    FROM legacy_collaboration_comments_v1 reaction
    WHERE reaction.legacy_video_id = video_alias.legacy_video_id
      AND reaction.comment_kind = 'emoji'
  ) AS total_reactions,
  COALESCE(owner.display_name, '') AS owner_name,
  COALESCE(folder.legacy_name, folder.name) AS folder_name,
  folder.legacy_color AS folder_color,
  CASE
    WHEN video.legacy_is_screenshot = 0
     AND EXISTS (SELECT 1 FROM video_uploads upload WHERE upload.video_id = video.id)
    THEN 1 ELSE 0
  END AS has_active_upload,
  video.legacy_effective_created_at_ms AS effective_created_at_ms
FROM videos video
JOIN organizations organization
  ON organization.id = video.organization_id
 AND organization.id = ?2
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = video.owner_id
LEFT JOIN users owner
  ON owner.id = video.owner_id
LEFT JOIN shared_videos placement
  ON placement.video_id = video.id
 AND placement.organization_id = ?2
 AND placement.revoked_at_ms IS NULL
LEFT JOIN folders folder
  ON folder.id = placement.folder_id
 AND folder.organization_id = ?2
 AND folder.deleted_at_ms IS NULL
WHERE video.owner_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
ORDER BY video.legacy_effective_created_at_us DESC;
