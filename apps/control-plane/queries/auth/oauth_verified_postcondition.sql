SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM auth_principal_issuance_grants_v2 grant_row
  JOIN auth_identities_v2 identity ON identity.user_id = grant_row.user_id
  JOIN users user_row ON user_row.id = identity.user_id AND user_row.status = 'active'
  WHERE grant_row.id = ?1
    AND grant_row.user_id = ?2
    AND grant_row.identity_revision = ?3
    AND grant_row.expires_at_ms = ?4
    AND grant_row.last_operation_id = ?5
) THEN 1 ELSE 0 END AS present
