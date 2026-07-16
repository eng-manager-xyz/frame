SELECT i.user_id,
       i.identity_revision,
       i.session_version,
       i.revision AS identity_row_revision,
       grant_row.organization_id,
       grant_row.role,
       grant_row.member_revision,
       grant_row.organization_revision
FROM auth_identities_v2 i
JOIN users u
  ON u.id = i.user_id
 AND u.status = 'active'
LEFT JOIN (
  SELECT m.user_id,
         m.organization_id,
         m.role,
         m.revision AS member_revision,
         o.revision AS organization_revision
  FROM organization_members m
  JOIN organizations o
    ON o.id = m.organization_id
   AND o.status = 'active'
  WHERE m.state = 'active'
) grant_row ON grant_row.user_id = i.user_id
WHERE i.user_id = ?1
ORDER BY grant_row.organization_id
