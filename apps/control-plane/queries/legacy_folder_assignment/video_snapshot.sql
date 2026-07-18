WITH requested(id) AS (
  SELECT CAST(value AS TEXT)
  FROM json_each(?1)
)
SELECT
  requested.id AS requested_id,
  v.id,
  v.owner_id,
  v.folder_id,
  v.revision
FROM requested
LEFT JOIN videos v
  ON v.id = requested.id
 AND v.organization_id = ?2
 AND v.deleted_at_ms IS NULL
ORDER BY requested.id
LIMIT 501
