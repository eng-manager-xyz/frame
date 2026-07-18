WITH requested(id) AS (
  SELECT CAST(value AS TEXT)
  FROM json_each(?1)
)
SELECT
  requested.id AS requested_id,
  membership.video_id,
  membership.folder_id,
  membership.revision
FROM requested
LEFT JOIN space_videos membership
  ON membership.space_id = ?2
 AND membership.video_id = requested.id
ORDER BY requested.id
LIMIT 501
