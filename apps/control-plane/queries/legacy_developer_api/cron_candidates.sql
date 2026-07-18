SELECT app.id AS app_id, account.id AS account_id, account.balance_microcredits,
       COALESCE(SUM(video.duration), 0.0) / 60.0 AS total_duration_minutes,
       COUNT(video.id) AS video_count
FROM legacy_developer_apps_v1 AS app
LEFT JOIN legacy_developer_credit_accounts_v1 AS account ON account.app_id = app.id
LEFT JOIN legacy_developer_videos_v1 AS video
  ON video.app_id = app.id AND video.deleted_at_ms IS NULL
LEFT JOIN legacy_developer_daily_storage_snapshots_v1 AS snapshot
  ON snapshot.app_id = app.id AND snapshot.snapshot_date = ?1
WHERE app.deleted_at_ms IS NULL AND snapshot.id IS NULL
GROUP BY app.id, account.id, account.balance_microcredits
ORDER BY app.id
