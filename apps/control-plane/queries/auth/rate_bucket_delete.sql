DELETE FROM auth_rate_limit_buckets_v2
WHERE action = ?1
  AND dimension = ?2
  AND key_version = ?3
  AND digest = ?4
  AND revision = ?5
