WITH RECURSIVE subtree(id, parent_id, depth, path) AS (
  SELECT id, parent_id, 0, '/' || id || '/'
  FROM folders
  WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL
  UNION ALL
  SELECT child.id, child.parent_id, subtree.depth + 1,
         subtree.path || child.id || '/'
  FROM folders child
  JOIN subtree ON child.parent_id = subtree.id
  WHERE child.organization_id = ?2
    AND child.deleted_at_ms IS NULL
    AND subtree.depth < 33
    AND instr(subtree.path, '/' || child.id || '/') = 0
)
SELECT
  COUNT(*) AS folder_count,
  COALESCE(MAX(depth), 0) AS max_depth,
  (SELECT json_group_array(id) FROM (SELECT id FROM subtree ORDER BY id)) AS ids_json
FROM subtree
