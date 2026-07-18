UPDATE developer_videos SET deleted_at_ms=?3, updated_at_ms=?3,
  revision=revision+1, last_operation_id=?4
WHERE id=?1 AND deleted_at_ms IS NULL AND created_at_ms<=?3 AND EXISTS (
  SELECT 1 FROM developer_apps app WHERE app.id=developer_videos.app_id AND app.organization_id=?2
)
