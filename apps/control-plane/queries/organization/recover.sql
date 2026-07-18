UPDATE organizations
SET status = 'active',
    tombstoned_at_ms = NULL,
    retention_until_ms = NULL,
    recovered_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    last_operation_id = ?3
WHERE id = ?1 AND status = 'tombstoned' AND tombstoned_at_ms = ?2
  AND retention_until_ms IS NOT NULL
  AND CAST(strftime('%s', 'now') AS INTEGER) * 1000 <= retention_until_ms
