WITH requested(id) AS (
  SELECT CAST(value AS TEXT)
  FROM json_each(?1)
)
SELECT
  requested.id AS requested_id,
  sv.video_id,
  sv.folder_id,
  sv.revision
FROM requested
LEFT JOIN space_videos sv
  ON sv.space_id = ?2
 AND sv.video_id = requested.id
ORDER BY requested.id
LIMIT 501
