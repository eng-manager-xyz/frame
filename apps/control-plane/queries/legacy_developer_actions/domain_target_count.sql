SELECT COUNT(*) AS target_count
FROM legacy_developer_app_domains_v1
WHERE id = ?1 AND app_id = ?2
