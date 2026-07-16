INSERT INTO organization_retention_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM organizations
         WHERE id = ?2 AND status = 'tombstoned'
           AND tombstoned_at_ms = ?3
           AND retention_until_ms IS NOT NULL
           AND CAST(strftime('%s', 'now') AS INTEGER) * 1000 <= retention_until_ms
       ) THEN 1 ELSE 0 END
