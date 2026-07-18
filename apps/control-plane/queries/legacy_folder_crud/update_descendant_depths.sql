WITH RECURSIVE descendants(id) AS (
  SELECT id FROM folders
  WHERE parent_id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL
  UNION
  SELECT child.id
  FROM folders child
  JOIN descendants parent ON child.parent_id = parent.id
  WHERE child.organization_id = ?2 AND child.deleted_at_ms IS NULL
)
UPDATE folders
SET depth = depth + ?3,
    revision = revision + 1,
    tree_revision = tree_revision + 1,
    updated_at_ms = ?4,
    last_operation_id = ?5
WHERE id IN (SELECT id FROM descendants)
