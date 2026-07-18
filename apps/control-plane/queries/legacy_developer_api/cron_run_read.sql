SELECT snapshot_date, apps_processed FROM legacy_developer_cron_runs_v1
WHERE snapshot_date = ?1 LIMIT 1
