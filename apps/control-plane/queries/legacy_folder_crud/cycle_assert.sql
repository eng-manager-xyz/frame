INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
WITH RECURSIVE ancestors(id, parent_id) AS (
  SELECT id, parent_id
  FROM folders
  WHERE id = ?2 AND organization_id = ?4 AND deleted_at_ms IS NULL
  UNION
  SELECT parent.id, parent.parent_id
  FROM folders parent
  JOIN ancestors child ON child.parent_id = parent.id
  WHERE parent.organization_id = ?4 AND parent.deleted_at_ms IS NULL
)
SELECT ?1, 'cycle', 0,
       COALESCE(SUM(CASE WHEN id = ?3 THEN 1 ELSE 0 END), 0)
FROM ancestors
