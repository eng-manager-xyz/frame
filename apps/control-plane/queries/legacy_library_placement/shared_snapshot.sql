WITH requested(id) AS (
  SELECT CAST(value AS TEXT)
  FROM json_each(?1)
), current_rows AS (
  SELECT shared.*
  FROM shared_videos shared
  JOIN videos video
    ON video.id = shared.video_id
   AND video.organization_id = ?2
   AND video.deleted_at_ms IS NULL
  WHERE shared.organization_id = ?2
    AND shared.revoked_at_ms IS NULL
), dormant_roots AS (
  SELECT shared.*
  FROM shared_videos shared
  JOIN videos video
    ON video.id = shared.video_id
   AND video.organization_id = ?2
   AND video.deleted_at_ms IS NULL
  WHERE shared.organization_id = ?2
    AND shared.revoked_at_ms IS NOT NULL
    AND shared.folder_id IS NULL
)
SELECT
  requested.id AS requested_id,
  (SELECT COUNT(*) FROM current_rows current WHERE current.video_id = requested.id) AS active_count,
  COALESCE((
    SELECT current.id FROM current_rows current
    WHERE current.video_id = requested.id
    ORDER BY current.id LIMIT 1
  ), '') AS active_id,
  (
    SELECT current.folder_id FROM current_rows current
    WHERE current.video_id = requested.id
    ORDER BY current.id LIMIT 1
  ) AS active_folder_id,
  COALESCE((
    SELECT current.sharing_mode FROM current_rows current
    WHERE current.video_id = requested.id
    ORDER BY current.id LIMIT 1
  ), '') AS active_sharing_mode,
  COALESCE((
    SELECT current.revision FROM current_rows current
    WHERE current.video_id = requested.id
    ORDER BY current.id LIMIT 1
  ), -1) AS active_revision,
  COALESCE((
    SELECT dormant.id FROM dormant_roots dormant
    WHERE dormant.video_id = requested.id
    ORDER BY dormant.revoked_at_ms DESC, dormant.id LIMIT 1
  ), '') AS dormant_root_id,
  COALESCE((
    SELECT dormant.revision FROM dormant_roots dormant
    WHERE dormant.video_id = requested.id
    ORDER BY dormant.revoked_at_ms DESC, dormant.id LIMIT 1
  ), -1) AS dormant_root_revision
FROM requested
ORDER BY requested.id
LIMIT 501
