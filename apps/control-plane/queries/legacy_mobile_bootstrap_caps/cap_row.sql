SELECT
  video.id AS mapped_video_id,
  video_alias.legacy_video_id AS legacy_video_id,
  video.title AS title,
  video.created_at_ms AS created_at_ms,
  video.updated_at_ms AS updated_at_ms,
  COALESCE(owner.display_name, '') AS owner_name,
  video.legacy_duration_seconds AS duration_seconds,
  folder.legacy_folder_id AS legacy_folder_id,
  video.legacy_public AS legacy_public,
  CASE WHEN video.legacy_password_hash IS NULL THEN 0 ELSE 1 END AS protected,
  COALESCE(media.view_count, 0) AS view_count,
  (
    SELECT COUNT(DISTINCT comment.legacy_comment_id)
    FROM legacy_collaboration_comments_v1 comment
    WHERE comment.legacy_video_id = video_alias.legacy_video_id
      AND comment.comment_kind = 'text'
  ) AS comment_count,
  (
    SELECT COUNT(DISTINCT reaction.legacy_comment_id)
    FROM legacy_collaboration_comments_v1 reaction
    WHERE reaction.legacy_video_id = video_alias.legacy_video_id
      AND reaction.comment_kind = 'emoji'
  ) AS reaction_count,
  upload.uploaded AS upload_uploaded,
  upload.total AS upload_total,
  upload.phase AS upload_phase,
  upload.processing_progress AS processing_progress,
  upload.processing_message AS processing_message,
  upload.processing_error AS processing_error,
  video.legacy_metadata_json AS metadata_json,
  media.transcription_status AS transcription_status,
  media.object_prefix AS object_prefix,
  media.source_type AS source_type,
  upload.raw_file_key AS raw_file_key,
  video.legacy_is_screenshot AS is_screenshot
FROM videos video
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
LEFT JOIN users owner ON owner.id = video.owner_id
LEFT JOIN folders folder
  ON folder.id = video.folder_id
LEFT JOIN legacy_mobile_cap_media_v1 media
  ON media.mapped_video_id = video.id
LEFT JOIN legacy_mobile_cap_uploads_v1 upload
  ON upload.mapped_video_id = video.id
WHERE video_alias.legacy_video_id = ?2
  AND video.owner_id = ?1
  AND video.deleted_at_ms IS NULL
  AND video.state <> 'deleted'
LIMIT 2;
