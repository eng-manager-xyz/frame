INSERT INTO auth_rate_limit_buckets_v2(
  action, dimension, key_version, digest,
  window_started_at_ms, attempt_count, blocked_until_ms,
  updated_at_ms, gc_at_ms, revision, last_operation_id
)
SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?11
WHERE ?10 = -1
   OR EXISTS (
    SELECT 1 FROM auth_rate_limit_buckets_v2
    WHERE action = ?1
      AND dimension = ?2
      AND key_version = ?3
      AND digest = ?4
      AND revision = ?10
  )
ON CONFLICT(action, dimension, key_version, digest) DO UPDATE SET
  window_started_at_ms = excluded.window_started_at_ms,
  attempt_count = excluded.attempt_count,
  blocked_until_ms = excluded.blocked_until_ms,
  updated_at_ms = excluded.updated_at_ms,
  gc_at_ms = excluded.gc_at_ms,
  revision = auth_rate_limit_buckets_v2.revision + 1,
  last_operation_id = excluded.last_operation_id
WHERE auth_rate_limit_buckets_v2.revision = ?10
