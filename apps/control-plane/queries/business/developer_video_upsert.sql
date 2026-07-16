INSERT INTO developer_videos(
  id, app_id, video_id, external_user_id, metadata_json, created_at_ms,
  updated_at_ms, deleted_at_ms, metadata_schema_version, metadata_checksum,
  revision, last_operation_id, external_user_digest
)
SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?13,?4
WHERE EXISTS (
  SELECT 1 FROM developer_apps app
  WHERE app.id = ?2 AND app.organization_id = ?14 AND app.status <> 'deleted'
)
ON CONFLICT(id) DO UPDATE SET
  video_id = excluded.video_id,
  metadata_json = excluded.metadata_json,
  updated_at_ms = excluded.updated_at_ms,
  deleted_at_ms = excluded.deleted_at_ms,
  metadata_schema_version = excluded.metadata_schema_version,
  metadata_checksum = excluded.metadata_checksum,
  revision = excluded.revision,
  last_operation_id = excluded.last_operation_id
WHERE developer_videos.app_id = excluded.app_id
  AND developer_videos.external_user_digest = excluded.external_user_digest
  AND developer_videos.created_at_ms = excluded.created_at_ms
  AND developer_videos.revision = ?12
