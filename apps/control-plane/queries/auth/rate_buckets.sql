SELECT action,
       dimension,
       key_version,
       digest,
       window_started_at_ms,
       attempt_count,
       blocked_until_ms,
       updated_at_ms,
       gc_at_ms,
       revision
FROM auth_rate_limit_buckets_v2 b
WHERE b.action = ?1
  AND b.dimension = ?2
  AND b.gc_at_ms > ?4
  AND (
    (?2 = 'global' AND b.key_version = 0 AND b.digest = '')
    OR EXISTS (
      SELECT 1
      FROM json_each(?3) candidate
      WHERE CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = b.key_version
        AND json_extract(candidate.value, '$.digest') = b.digest
    )
  )
ORDER BY b.updated_at_ms DESC
LIMIT 5
