INSERT INTO videos(
  id, owner_id, title, state, created_at_ms, updated_at_ms,
  organization_id, privacy, metadata_json, metadata_schema_version,
  metadata_checksum, comments_enabled, revision, deleted_at_ms, last_operation_id
) VALUES (?1,?2,'Untitled','pending',?9,?10,?3,?4,?5,?6,?7,?8,?11,NULL,?13)
ON CONFLICT(id) DO UPDATE SET
  privacy = excluded.privacy,
  metadata_json = excluded.metadata_json,
  metadata_schema_version = excluded.metadata_schema_version,
  metadata_checksum = excluded.metadata_checksum,
  comments_enabled = excluded.comments_enabled,
  updated_at_ms = excluded.updated_at_ms,
  revision = excluded.revision,
  deleted_at_ms = NULL,
  last_operation_id = excluded.last_operation_id
WHERE videos.organization_id = excluded.organization_id
  AND videos.owner_id = excluded.owner_id
  AND videos.created_at_ms = excluded.created_at_ms
  AND videos.revision = ?12
