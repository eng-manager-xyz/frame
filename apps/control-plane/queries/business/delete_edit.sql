DELETE FROM video_edits
WHERE id=?1 AND EXISTS (
  SELECT 1 FROM videos WHERE videos.id=video_edits.video_id AND videos.organization_id=?2
) AND ?3>=0 AND length(?4)=36
