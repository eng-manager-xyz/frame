SELECT session.provider_upload_id, session.object_key, session.content_type, session.state,
       session.video_id AS native_video_id, video.legacy_video_id
FROM legacy_developer_multipart_sessions_v1 AS session
JOIN legacy_developer_videos_v1 AS video ON video.id = session.video_id
WHERE session.app_id = ?1 AND session.provider_upload_id = ?2
  AND video.legacy_video_id = ?3 AND video.deleted_at_ms IS NULL
LIMIT 1
