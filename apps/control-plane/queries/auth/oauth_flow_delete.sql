DELETE FROM auth_oauth_flows_v2
WHERE id = ?1
  AND revision = ?2
