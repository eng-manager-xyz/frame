DELETE FROM space_members
WHERE user_id = ?1
  AND space_id IN (
    SELECT id FROM spaces
    WHERE organization_id = ?2
  )
  AND ?3 = 1;
