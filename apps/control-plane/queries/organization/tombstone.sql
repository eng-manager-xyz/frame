UPDATE organizations
SET status = 'tombstoned',
    tombstoned_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    retention_until_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000 + ?2,
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    last_operation_id = ?3
WHERE id = ?1
  AND status = 'active'
  AND ?2 BETWEEN 1 AND 9007199254740991
  AND CAST(strftime('%s', 'now') AS INTEGER) * 1000 + ?2 <= 9007199254740991
