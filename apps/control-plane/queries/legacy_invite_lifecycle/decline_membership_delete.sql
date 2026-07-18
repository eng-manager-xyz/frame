DELETE FROM organization_members
WHERE organization_id = ?1
  AND user_id = ?2
  AND state = 'active'
  AND ?3 = 1;
