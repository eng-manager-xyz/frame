INSERT INTO storage_objects(
  id, organization_id, integration_id, video_id, object_key, role,
  object_version, state, bytes, content_type, checksum_sha256, provider_etag,
  created_at_ms, deleted_at_ms, updated_at_ms, revision, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,?12,?13,?14,?15,?17)
ON CONFLICT(id) DO UPDATE SET
  state = excluded.state,
  bytes = excluded.bytes,
  content_type = excluded.content_type,
  checksum_sha256 = excluded.checksum_sha256,
  deleted_at_ms = excluded.deleted_at_ms,
  updated_at_ms = excluded.updated_at_ms,
  revision = excluded.revision,
  last_operation_id = excluded.last_operation_id
WHERE storage_objects.organization_id = excluded.organization_id
  AND storage_objects.integration_id = excluded.integration_id
  AND storage_objects.video_id IS excluded.video_id
  AND storage_objects.object_key = excluded.object_key
  AND storage_objects.role = excluded.role
  AND storage_objects.object_version = excluded.object_version
  AND storage_objects.created_at_ms = excluded.created_at_ms
  AND storage_objects.revision = ?16
