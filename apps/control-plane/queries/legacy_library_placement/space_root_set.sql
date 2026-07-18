UPDATE space_videos
SET folder_id = NULL,
    revision = revision + 1,
    last_operation_id = ?3
WHERE space_id = ?1
  AND video_id = ?2
  AND revision = ?4
  AND folder_id IS ?5
