WITH requested(id) AS (
  SELECT CAST(value AS TEXT)
  FROM json_each(?1)
)
SELECT
  requested.id AS requested_id,
  (
    SELECT COUNT(*) FROM shared_videos current
    WHERE current.organization_id = ?2
      AND current.video_id = requested.id
      AND current.revoked_at_ms IS NULL
  ) AS active_count,
  COALESCE((
    SELECT current.id FROM shared_videos current
    WHERE current.organization_id = ?2
      AND current.video_id = requested.id
      AND current.revoked_at_ms IS NULL
    ORDER BY current.id LIMIT 1
  ), '') AS active_id,
  (
    SELECT current.folder_id FROM shared_videos current
    WHERE current.organization_id = ?2
      AND current.video_id = requested.id
      AND current.revoked_at_ms IS NULL
    ORDER BY current.id LIMIT 1
  ) AS active_folder_id,
  COALESCE((
    SELECT current.revision FROM shared_videos current
    WHERE current.organization_id = ?2
      AND current.video_id = requested.id
      AND current.revoked_at_ms IS NULL
    ORDER BY current.id LIMIT 1
  ), -1) AS active_revision,
  COALESCE((
    SELECT dormant.id FROM shared_videos dormant
    WHERE dormant.organization_id = ?2
      AND dormant.video_id = requested.id
      AND dormant.revoked_at_ms IS NOT NULL
      AND dormant.folder_id IS ?3
    ORDER BY dormant.revoked_at_ms DESC, dormant.id
    LIMIT 1
  ), '') AS dormant_id,
  COALESCE((
    SELECT dormant.revision FROM shared_videos dormant
    WHERE dormant.organization_id = ?2
      AND dormant.video_id = requested.id
      AND dormant.revoked_at_ms IS NOT NULL
      AND dormant.folder_id IS ?3
    ORDER BY dormant.revoked_at_ms DESC, dormant.id
    LIMIT 1
  ), -1) AS dormant_revision
FROM requested
ORDER BY requested.id
LIMIT 501
