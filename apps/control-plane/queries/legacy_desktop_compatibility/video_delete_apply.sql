UPDATE videos
SET state = 'deleted',
    deleted_at_ms = ?4,
    updated_at_ms = ?4,
    revision = revision + 1,
    last_operation_id = ?5
WHERE id = ?1
  AND owner_id = ?2
  AND organization_id = ?3
  AND state <> 'deleted'
  AND deleted_at_ms IS NULL
  AND revision = ?6;
