INSERT INTO developer_daily_storage_snapshots(
  app_id, snapshot_day, total_bytes, microcredits_charged, source_checksum,
  processed_at_ms, created_at_ms, revision, last_operation_id
)
SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?10
WHERE EXISTS (
  SELECT 1 FROM developer_apps app
  WHERE app.id = ?1 AND app.organization_id = ?11
)
ON CONFLICT(app_id, snapshot_day) DO UPDATE SET
  total_bytes = excluded.total_bytes,
  microcredits_charged = excluded.microcredits_charged,
  source_checksum = excluded.source_checksum,
  processed_at_ms = excluded.processed_at_ms,
  revision = excluded.revision,
  last_operation_id = excluded.last_operation_id
WHERE developer_daily_storage_snapshots.created_at_ms = excluded.created_at_ms
  AND developer_daily_storage_snapshots.revision = ?9
