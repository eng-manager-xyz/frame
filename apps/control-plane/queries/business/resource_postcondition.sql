INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN (
         (?2 = 'video' AND EXISTS (SELECT 1 FROM videos WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'edit' AND EXISTS (SELECT 1 FROM video_edits e JOIN videos v ON v.id=e.video_id WHERE e.id=?3 AND v.organization_id=?4 AND e.revision=?5 AND e.last_operation_id=?6))
         OR (?2 = 'share' AND EXISTS (SELECT 1 FROM shared_videos WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'comment' AND EXISTS (SELECT 1 FROM comments WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'storage_object' AND EXISTS (SELECT 1 FROM storage_objects WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'storage_integration' AND EXISTS (SELECT 1 FROM storage_integrations WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'derivative_job' AND EXISTS (SELECT 1 FROM business_derivative_manifests_v1 WHERE job_id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'import' AND EXISTS (SELECT 1 FROM imported_videos WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'upload' AND EXISTS (SELECT 1 FROM video_uploads WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6))
         OR (?2 = 'developer_key' AND EXISTS (
           SELECT 1 FROM developer_api_keys k JOIN developer_apps a ON a.id=k.app_id
           WHERE k.id=?3 AND a.organization_id=?4 AND k.revision=?5 AND k.last_operation_id=?6
         ))
         OR (?2 = 'developer_app' AND EXISTS (
           SELECT 1 FROM developer_apps
           WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6
         ))
         OR (?2 = 'developer_domain' AND EXISTS (
           SELECT 1 FROM developer_app_domains d JOIN developer_apps a ON a.id=d.app_id
           WHERE d.app_id || ':' || d.domain_ascii=?3 AND a.organization_id=?4
             AND d.revision=?5 AND d.last_operation_id=?6
         ))
         OR (?2 = 'developer_video' AND EXISTS (
           SELECT 1 FROM developer_videos d JOIN developer_apps a ON a.id=d.app_id
           WHERE d.id=?3 AND a.organization_id=?4 AND d.revision=?5 AND d.last_operation_id=?6
         ))
         OR (?2 = 'credit_transaction' AND EXISTS (
           SELECT 1 FROM developer_credit_transactions t
           JOIN developer_credit_accounts c ON c.id=t.account_id
           JOIN developer_apps a ON a.id=c.app_id
           WHERE t.id=?3 AND a.organization_id=?4 AND t.operation_id=?6
         ))
         OR (?2 = 'usage' AND EXISTS (
           SELECT 1 FROM usage_ledger u LEFT JOIN developer_apps a ON a.id=u.app_id
           WHERE u.id=?3 AND (u.organization_id=?4 OR a.organization_id=?4) AND u.operation_id=?6
         ))
         OR (?2 = 'daily_snapshot' AND EXISTS (
           SELECT 1 FROM developer_daily_storage_snapshots s JOIN developer_apps a ON a.id=s.app_id
           WHERE s.app_id=?3 AND a.organization_id=?4 AND s.revision=?5 AND s.last_operation_id=?6
         ))
         OR (?2 = 'notification' AND EXISTS (
           SELECT 1 FROM notifications WHERE id=?3 AND organization_id=?4 AND last_operation_id=?6
         ))
         OR (?2 = 'outbox' AND EXISTS (
           SELECT 1 FROM outbox_events WHERE id=?3 AND organization_id=?4 AND revision=?5 AND last_operation_id=?6
         ))
         OR (?2 = 'data_request' AND EXISTS (
           SELECT 1 FROM business_data_requests_v1 WHERE id=?3 AND organization_id=?4 AND operation_id=?6
         ))
       ) THEN 1 ELSE 0 END
