INSERT OR IGNORE INTO legacy_analytics_password_grants_v1(
  grant_digest, video_id, issued_at_ms, expires_at_ms
)
SELECT DISTINCT ?3, video.id, ?4, ?5
FROM videos video
WHERE video.deleted_at_ms IS NULL
  AND (
    EXISTS (
      SELECT 1 FROM json_each(?1) requested
      WHERE requested.type = 'text' AND requested.value = video.id
    )
    OR EXISTS (
      SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
      JOIN json_each(?1) requested
        ON requested.type = 'text' AND requested.value = alias.legacy_video_id
      WHERE alias.mapped_video_id = video.id
    )
  )
  AND (
    EXISTS (
      SELECT 1 FROM json_each(?2) candidate
      WHERE candidate.type = 'text' AND candidate.value = video.legacy_password_hash
    )
    OR EXISTS (
      SELECT 1 FROM space_videos placement
      JOIN spaces space ON space.id = placement.space_id AND space.deleted_at_ms IS NULL
      JOIN json_each(?2) candidate
        ON candidate.type = 'text' AND candidate.value = space.legacy_password_hash
      WHERE placement.video_id = video.id
    )
  )
