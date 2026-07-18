SELECT COUNT(*) AS bucket_count
FROM auth_rate_limit_buckets_v2
WHERE gc_at_ms > ?1
