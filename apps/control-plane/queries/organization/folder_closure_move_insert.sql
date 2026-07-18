INSERT INTO organization_folder_closure_v1(
  organization_id, space_id, ancestor_id, descendant_id, distance
)
SELECT ?1, ?2, supertree.ancestor_id, subtree.descendant_id,
       supertree.distance + subtree.distance + 1
FROM organization_folder_closure_v1 supertree
JOIN organization_folder_closure_v1 subtree
  ON subtree.organization_id = ?1 AND subtree.space_id = ?2 AND subtree.ancestor_id = ?3
WHERE supertree.organization_id = ?1 AND supertree.space_id = ?2 AND supertree.descendant_id = ?4
