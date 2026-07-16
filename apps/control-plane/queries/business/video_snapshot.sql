SELECT id, owner_id, organization_id, privacy, metadata_json,
       metadata_schema_version, metadata_checksum, comments_enabled,
       created_at_ms, updated_at_ms, deleted_at_ms, revision
FROM videos
WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL
LIMIT 2
