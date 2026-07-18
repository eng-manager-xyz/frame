SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_oauth_flows_v2 flow
  WHERE flow.id = ?1
    AND flow.expires_at_ms > ?2
) THEN 1 ELSE 0 END AS present
