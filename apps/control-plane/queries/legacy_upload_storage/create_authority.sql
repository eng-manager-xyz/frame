SELECT
  actor.id AS actor_id,
  actor_alias.legacy_user_id AS legacy_actor_id,
  organization.id AS organization_id,
  organization_alias.legacy_organization_id AS legacy_organization_id,
  storage.id AS storage_integration_id,
  COALESCE(member.has_pro_seat, 0) AS has_pro_seat,
  folder.id AS folder_id,
  folder.legacy_folder_id AS legacy_folder_id
FROM users actor
JOIN legacy_collaboration_user_aliases_v1 actor_alias
  ON actor_alias.mapped_user_id = actor.id
JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.legacy_organization_id = ?2
JOIN organizations organization
  ON organization.id = organization_alias.organization_id AND organization.status = 'active'
LEFT JOIN organization_members member
  ON member.organization_id = organization.id AND member.user_id = actor.id AND member.state = 'active'
JOIN storage_integrations storage
  ON storage.organization_id = organization.id
 AND storage.provider = 'r2' AND storage.state = 'active'
 AND json_extract(storage.capabilities_json, '$.single_put') = 1
LEFT JOIN folders folder
  ON ?3 IS NOT NULL AND folder.legacy_folder_id = ?3
 AND folder.organization_id = organization.id AND folder.deleted_at_ms IS NULL
WHERE actor.id = ?1 AND actor.status = 'active' AND actor.deleted_at_ms IS NULL
  AND (organization.owner_id = actor.id OR member.user_id IS NOT NULL)
  AND (?3 IS NULL OR folder.id IS NOT NULL)
ORDER BY storage.updated_at_ms DESC, storage.id
LIMIT 2;
