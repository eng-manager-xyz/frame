INSERT INTO legacy_folder_crud_delete_targets_v1(operation_id, folder_id, depth)
WITH RECURSIVE subtree(id, depth, path) AS (
  SELECT id, 0, '/' || id || '/'
  FROM folders
  WHERE id = ?2 AND organization_id = ?3 AND deleted_at_ms IS NULL
  UNION ALL
  SELECT child.id, subtree.depth + 1, subtree.path || child.id || '/'
  FROM folders child
  JOIN subtree ON child.parent_id = subtree.id
  WHERE child.organization_id = ?3
    AND child.deleted_at_ms IS NULL
    AND subtree.depth < 32
    AND instr(subtree.path, '/' || child.id || '/') = 0
)
SELECT ?1, id, depth FROM subtree
