SELECT
  job.id,
  job.video_id,
  job.state,
  job.attempt,
  job.updated_at_ms,
  job.lease_expires_at_ms
FROM media_jobs job
JOIN videos video ON video.id = job.video_id
WHERE job.id = ?1
  AND video.deleted_at_ms IS NULL
LIMIT 2;
