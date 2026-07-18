DELETE FROM compatibility_rate_limit_buckets_v1
WHERE rowid IN (
  SELECT rowid
  FROM compatibility_rate_limit_buckets_v1
  WHERE gc_at_ms <= ?1
  ORDER BY gc_at_ms, rowid
  LIMIT 16
)
