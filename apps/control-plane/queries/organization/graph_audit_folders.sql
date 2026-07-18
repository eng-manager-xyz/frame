SELECT finding_kind, subject_id, observed_revision
FROM (
  SELECT 'folder_without_space' AS finding_kind,
         f.id AS subject_id,
         f.revision AS observed_revision
  FROM folders f LEFT JOIN spaces s ON s.id = f.space_id
  WHERE f.organization_id = ?1 AND f.deleted_at_ms IS NULL
    AND (s.id IS NULL OR s.organization_id <> f.organization_id)

  UNION ALL
  SELECT 'folder_crosses_space', f.id, f.revision
  FROM folders f JOIN folders p ON p.id = f.parent_id
  WHERE f.organization_id = ?1
    AND (p.organization_id <> f.organization_id OR p.space_id <> f.space_id)

  UNION ALL
  SELECT DISTINCT 'folder_cycle', f.id, f.revision
  FROM folders f JOIN organization_folder_closure_v1 c
    ON c.organization_id = f.organization_id AND c.space_id = f.space_id
   AND c.ancestor_id = f.id AND c.descendant_id = f.id AND c.distance <> 0
  WHERE f.organization_id = ?1

  UNION ALL
  SELECT 'folder_depth_mismatch', f.id, f.revision
  FROM folders f
  WHERE f.organization_id = ?1 AND f.deleted_at_ms IS NULL
    AND f.depth <> (
      SELECT COUNT(*) FROM organization_folder_closure_v1 c
      WHERE c.organization_id = f.organization_id AND c.space_id = f.space_id
        AND c.descendant_id = f.id AND c.ancestor_id <> f.id
    )

  UNION ALL
  SELECT DISTINCT 'deleted_ancestor', f.id, f.revision
  FROM folders f JOIN organization_folder_closure_v1 c
    ON c.organization_id = f.organization_id AND c.space_id = f.space_id AND c.descendant_id = f.id
  JOIN folders ancestor ON ancestor.id = c.ancestor_id
  WHERE f.organization_id = ?1 AND f.deleted_at_ms IS NULL
    AND ancestor.deleted_at_ms IS NOT NULL
)
ORDER BY finding_kind, subject_id
LIMIT ?2
