SELECT video.legacy_video_id, app.legacy_app_id, video.external_user_id, video.name,
       video.duration, video.width, video.height, video.fps, video.s3_key,
       video.transcription_status, video.metadata_json, video.deleted_at_ms,
       video.created_at_ms, video.updated_at_ms
FROM legacy_developer_videos_v1 AS video
JOIN legacy_developer_apps_v1 AS app ON app.id = video.app_id
WHERE video.app_id = ?1 AND video.deleted_at_ms IS NULL
  AND (?2 IS NULL OR video.external_user_id = ?2)
ORDER BY video.created_at_ms DESC, video.id DESC
LIMIT ?3 OFFSET ?4
