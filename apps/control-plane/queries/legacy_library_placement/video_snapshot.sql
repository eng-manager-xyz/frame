WITH requested(id) AS (
  SELECT CAST(value AS TEXT)
  FROM json_each(?1)
)
SELECT
  requested.id AS requested_id,
  v.id,
  v.owner_id,
  v.folder_id,
  v.revision,
  CASE WHEN v.folder_id IS NOT NULL AND EXISTS (
    SELECT 1
    FROM folders f
    LEFT JOIN spaces s
      ON s.id = f.space_id
     AND s.organization_id = f.organization_id
     AND s.deleted_at_ms IS NULL
    WHERE f.id = v.folder_id
      AND f.organization_id = ?2
      AND f.deleted_at_ms IS NULL
      AND (f.space_id IS NULL OR s.id IS NOT NULL)
  ) THEN 1 ELSE 0 END AS folder_in_tenant
FROM requested
LEFT JOIN videos v
  ON v.id = requested.id
 AND v.organization_id = ?2
 AND v.deleted_at_ms IS NULL
ORDER BY requested.id
LIMIT 501
