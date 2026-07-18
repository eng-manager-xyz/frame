UPDATE videos
SET state = 'deleted',
    deleted_at_ms = ?3,
    updated_at_ms = MAX(updated_at_ms, ?3),
    revision = revision + 1,
    last_operation_id = ?4
WHERE id = ?1
  AND owner_id = ?2
  AND deleted_at_ms IS NULL
  AND state <> 'deleted';
