SELECT CASE WHEN
  video.owner_id = ?2
  OR EXISTS (
    SELECT 1
    FROM organization_members member
    WHERE member.organization_id = video.organization_id
      AND member.user_id = ?2
      AND member.state = 'active'
  )
  OR EXISTS (
    SELECT 1
    FROM space_videos placement
    JOIN space_members member ON member.space_id = placement.space_id
    JOIN spaces space ON space.id = placement.space_id
    WHERE placement.video_id = video.id
      AND member.user_id = ?2
      AND space.deleted_at_ms IS NULL
  )
  THEN 1 ELSE 0 END AS allowed
FROM videos video
WHERE video.id = ?1
  AND video.deleted_at_ms IS NULL
LIMIT 2;
