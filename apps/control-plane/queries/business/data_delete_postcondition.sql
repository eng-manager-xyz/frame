INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN
  (?3='video_metadata' AND EXISTS (SELECT 1 FROM videos WHERE id=?4 AND organization_id=?2 AND deleted_at_ms=?5 AND last_operation_id=?6))
  OR (?3='video_edit' AND NOT EXISTS (SELECT 1 FROM video_edits item JOIN videos video ON video.id=item.video_id WHERE item.id=?4 AND video.organization_id=?2))
  OR (?3='share' AND EXISTS (SELECT 1 FROM shared_videos WHERE id=?4 AND organization_id=?2 AND revoked_at_ms=?5 AND last_operation_id=?6))
  OR (?3='comment' AND EXISTS (SELECT 1 FROM comments WHERE id=?4 AND organization_id=?2 AND deleted_at_ms=?5 AND last_operation_id=?6))
  OR (?3='notification' AND NOT EXISTS (SELECT 1 FROM notifications WHERE id=?4 AND organization_id=?2))
  OR (?3='outbox' AND NOT EXISTS (SELECT 1 FROM outbox_events WHERE id=?4 AND organization_id=?2))
  OR (?3='storage_integration' AND EXISTS (SELECT 1 FROM storage_integrations WHERE id=?4 AND organization_id=?2 AND state='revoked' AND credential_ciphertext IS NULL AND last_operation_id=?6))
  OR (?3='storage_object' AND EXISTS (SELECT 1 FROM storage_objects WHERE id=?4 AND organization_id=?2 AND state='deleted' AND deleted_at_ms=?5 AND last_operation_id=?6))
  OR (?3='derivative_job' AND NOT EXISTS (SELECT 1 FROM business_derivative_manifests_v1 WHERE job_id=?4 AND organization_id=?2))
  OR (?3='upload' AND NOT EXISTS (SELECT 1 FROM video_uploads WHERE id=?4 AND organization_id=?2))
  OR (?3='import' AND NOT EXISTS (SELECT 1 FROM imported_videos WHERE id=?4 AND organization_id=?2))
  OR (?3='developer_app' AND EXISTS (SELECT 1 FROM developer_apps WHERE id=?4 AND organization_id=?2 AND status='deleted' AND deleted_at_ms=?5 AND last_operation_id=?6))
  OR (?3='developer_domain' AND NOT EXISTS (SELECT 1 FROM developer_app_domains item JOIN developer_apps app ON app.id=item.app_id WHERE item.app_id || ':' || item.domain_ascii=?4 AND app.organization_id=?2))
  OR (?3='developer_api_key' AND NOT EXISTS (SELECT 1 FROM developer_api_keys item JOIN developer_apps app ON app.id=item.app_id WHERE item.id=?4 AND app.organization_id=?2))
  OR (?3='developer_video' AND EXISTS (SELECT 1 FROM developer_videos item JOIN developer_apps app ON app.id=item.app_id WHERE item.id=?4 AND app.organization_id=?2 AND item.deleted_at_ms=?5 AND item.last_operation_id=?6))
  OR (?3='credit_account' AND EXISTS (SELECT 1 FROM developer_credit_accounts item JOIN developer_apps app ON app.id=item.app_id WHERE item.id=?4 AND app.organization_id=?2))
  OR (?3='daily_storage_snapshot' AND EXISTS (SELECT 1 FROM developer_daily_storage_snapshots item JOIN developer_apps app ON app.id=item.app_id WHERE item.app_id || ':' || item.snapshot_day=?4 AND app.organization_id=?2))
  OR (?3='messenger_legacy' AND EXISTS (
    SELECT 1 FROM business_messenger_legacy_quarantine_v1 item
    WHERE item.organization_id=?2 AND item.source_table || ':' || item.source_id=?4
      AND item.disposition='purged' AND item.purged_at_ms=?5 AND item.last_operation_id=?6
      AND NOT EXISTS (SELECT 1 FROM messenger_conversations source WHERE item.source_table='messenger_conversations' AND source.id=item.source_id)
      AND NOT EXISTS (SELECT 1 FROM messenger_messages source WHERE item.source_table='messenger_messages' AND source.id=item.source_id)
      AND NOT EXISTS (SELECT 1 FROM messenger_support_emails source WHERE item.source_table='messenger_support_emails' AND source.id=item.source_id)
  ))
THEN 1 ELSE 0 END
