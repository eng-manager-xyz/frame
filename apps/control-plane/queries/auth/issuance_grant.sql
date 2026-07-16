SELECT g.id,
       g.user_id,
       g.identity_revision,
       g.expires_at_ms,
       i.identity_revision AS current_identity_revision
FROM auth_principal_issuance_grants_v2 g
JOIN auth_identities_v2 i ON i.user_id = g.user_id
JOIN users u ON u.id = g.user_id AND u.status = 'active'
WHERE g.id = ?1
LIMIT 1
