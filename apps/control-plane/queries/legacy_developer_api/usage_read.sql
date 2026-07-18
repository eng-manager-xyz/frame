SELECT account.balance_microcredits,
       (SELECT COUNT(*) FROM legacy_developer_videos_v1 video
        WHERE video.app_id = ?1 AND video.deleted_at_ms IS NULL) AS total_videos,
       COALESCE((SELECT SUM(video.duration) FROM legacy_developer_videos_v1 video
        WHERE video.app_id = ?1 AND video.deleted_at_ms IS NULL), 0.0) / 60.0
         AS total_duration_minutes
FROM legacy_developer_credit_accounts_v1 AS account
WHERE account.app_id = ?1
LIMIT 1
