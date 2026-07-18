SELECT
  organization.id AS organization_id,
  storage.id AS storage_integration_id,
  (
    SELECT folder.id
    FROM folders folder
    WHERE ?3 IS NOT NULL
      AND (folder.legacy_folder_id = ?3 OR folder.id = ?3)
      AND folder.organization_id = organization.id
      AND folder.deleted_at_ms IS NULL
    ORDER BY CASE WHEN folder.legacy_folder_id = ?3 THEN 0 ELSE 1 END, folder.id
    LIMIT 1
  ) AS folder_id,
  domain.custom_domain,
  CASE WHEN domain.domain_verified_iso IS NULL THEN 0 ELSE 1 END AS domain_verified
FROM users actor
JOIN organizations organization
  ON organization.id = ?2 AND organization.status = 'active'
JOIN storage_integrations storage
  ON storage.organization_id = organization.id
 AND storage.provider = 'r2' AND storage.state = 'active'
LEFT JOIN legacy_org_custom_domain_projection_v1 domain
  ON domain.organization_id = organization.id
WHERE actor.id = ?1
  AND actor.active_organization_id = organization.id
  AND (
    organization.owner_id = actor.id
    OR EXISTS (
      SELECT 1 FROM organization_members member
      WHERE member.organization_id = organization.id
        AND member.user_id = actor.id AND member.state = 'active'
    )
  )
  AND (
    ?3 IS NULL
    OR EXISTS (
      SELECT 1 FROM folders folder
      WHERE (folder.legacy_folder_id = ?3 OR folder.id = ?3)
        AND folder.organization_id = organization.id
        AND folder.deleted_at_ms IS NULL
    )
  )
ORDER BY storage.created_at_ms, storage.id
LIMIT 2;
