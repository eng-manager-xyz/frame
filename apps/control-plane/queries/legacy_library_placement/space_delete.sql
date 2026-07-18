DELETE FROM space_videos
WHERE space_id = ?1
  AND video_id = ?2
  AND revision = ?3
