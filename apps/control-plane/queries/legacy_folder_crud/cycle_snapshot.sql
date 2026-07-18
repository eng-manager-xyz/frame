WITH RECURSIVE ancestors(id, parent_id) AS (
  SELECT id, parent_id
  FROM folders
  WHERE id = ?1 AND organization_id = ?3 AND deleted_at_ms IS NULL
  UNION
  SELECT parent.id, parent.parent_id
  FROM folders parent
  JOIN ancestors child ON child.parent_id = parent.id
  WHERE parent.organization_id = ?3 AND parent.deleted_at_ms IS NULL
)
SELECT
  COALESCE(SUM(CASE WHEN id = ?2 THEN 1 ELSE 0 END), 0) AS cycle_count,
  COUNT(*) AS ancestor_count
FROM ancestors
