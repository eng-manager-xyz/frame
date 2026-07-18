SELECT 'video_metadata' AS data_class, COUNT(*) AS item_count FROM videos WHERE organization_id = ?1
UNION ALL SELECT 'video_edit', COUNT(*) FROM video_edits e JOIN videos v ON v.id=e.video_id WHERE v.organization_id=?1
UNION ALL SELECT 'share', COUNT(*) FROM shared_videos WHERE organization_id=?1
UNION ALL SELECT 'comment', COUNT(*) FROM comments WHERE organization_id=?1
UNION ALL SELECT 'notification', COUNT(*) FROM notifications WHERE organization_id=?1
UNION ALL SELECT 'outbox', COUNT(*) FROM outbox_events WHERE organization_id=?1
UNION ALL SELECT 'storage_integration', COUNT(*) FROM storage_integrations WHERE organization_id=?1
UNION ALL SELECT 'storage_object', COUNT(*) FROM storage_objects WHERE organization_id=?1
UNION ALL SELECT 'derivative_job', COUNT(*) FROM business_derivative_manifests_v1 WHERE organization_id=?1
UNION ALL SELECT 'upload', COUNT(*) FROM video_uploads WHERE organization_id=?1
UNION ALL SELECT 'import', COUNT(*) FROM imported_videos WHERE organization_id=?1
UNION ALL SELECT 'developer_app', COUNT(*) FROM developer_apps WHERE organization_id=?1
UNION ALL SELECT 'developer_domain', COUNT(*) FROM developer_app_domains d JOIN developer_apps a ON a.id=d.app_id WHERE a.organization_id=?1
UNION ALL SELECT 'developer_api_key', COUNT(*) FROM developer_api_keys k JOIN developer_apps a ON a.id=k.app_id WHERE a.organization_id=?1
UNION ALL SELECT 'developer_video', COUNT(*) FROM developer_videos d JOIN developer_apps a ON a.id=d.app_id WHERE a.organization_id=?1
UNION ALL SELECT 'credit_account', COUNT(*) FROM developer_credit_accounts c JOIN developer_apps a ON a.id=c.app_id WHERE a.organization_id=?1
UNION ALL SELECT 'credit_transaction', COUNT(*) FROM developer_credit_transactions t JOIN developer_credit_accounts c ON c.id=t.account_id JOIN developer_apps a ON a.id=c.app_id WHERE a.organization_id=?1
UNION ALL SELECT 'usage_ledger', COUNT(*) FROM usage_ledger u LEFT JOIN developer_apps a ON a.id=u.app_id WHERE u.organization_id=?1 OR a.organization_id=?1
UNION ALL SELECT 'daily_storage_snapshot', COUNT(*) FROM developer_daily_storage_snapshots s JOIN developer_apps a ON a.id=s.app_id WHERE a.organization_id=?1
UNION ALL SELECT 'messenger_legacy', 0
