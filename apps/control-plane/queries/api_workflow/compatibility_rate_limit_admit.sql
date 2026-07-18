INSERT INTO compatibility_rate_limit_buckets_v1(
  bucket, dimension, key_version, subject_digest,
  window_started_at_ms, request_count, updated_at_ms, gc_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, 1, ?5, ?6)
ON CONFLICT(bucket, dimension, key_version, subject_digest) DO UPDATE SET
  window_started_at_ms = CASE
    WHEN compatibility_rate_limit_buckets_v1.window_started_at_ms <= ?7 THEN ?5
    ELSE compatibility_rate_limit_buckets_v1.window_started_at_ms
  END,
  request_count = CASE
    WHEN compatibility_rate_limit_buckets_v1.window_started_at_ms <= ?7 THEN 1
    ELSE compatibility_rate_limit_buckets_v1.request_count + 1
  END,
  updated_at_ms = ?5,
  gc_at_ms = ?6
WHERE compatibility_rate_limit_buckets_v1.updated_at_ms <= ?5
  AND (
    compatibility_rate_limit_buckets_v1.window_started_at_ms <= ?7
    OR compatibility_rate_limit_buckets_v1.request_count < ?8
  )
RETURNING request_count
