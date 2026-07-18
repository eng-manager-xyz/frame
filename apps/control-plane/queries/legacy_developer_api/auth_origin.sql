SELECT EXISTS(
  SELECT 1 FROM legacy_developer_app_domains_v1
  WHERE app_id = ?1 AND origin = ?2
) AS allowed
