SELECT legacy_organization_id
FROM legacy_user_account_organization_ids_v1
WHERE organization_id = ?1
LIMIT 1;
