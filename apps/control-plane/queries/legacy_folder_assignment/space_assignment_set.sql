UPDATE space_videos
SET folder_id = ?3,
    revision = revision + 1,
    last_operation_id = ?4
WHERE space_id = ?1
  AND video_id = ?2
  AND revision = ?5
  AND folder_id IS ?6
