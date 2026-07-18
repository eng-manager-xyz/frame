INSERT INTO organization_folder_closure_v1(
  organization_id, space_id, ancestor_id, descendant_id, distance
)
SELECT ?2, ?3, ?1, ?1, 0
UNION ALL
SELECT organization_id, space_id, ancestor_id, ?1, distance + 1
FROM organization_folder_closure_v1
WHERE organization_id = ?2 AND space_id = ?3 AND descendant_id = ?4
