SELECT
  actor.id AS actor_id,
  actor_alias.legacy_user_id AS legacy_actor_id,
  COALESCE(actor.display_name, '') AS owner_name,
  organization.id AS organization_id,
  organization_alias.legacy_organization_id AS legacy_organization_id,
  storage.id AS storage_integration_id,
  (
    SELECT folder.id
    FROM folders folder
    WHERE ?3 IS NOT NULL
      AND folder.legacy_folder_id = ?3
      AND folder.organization_id = organization.id
      AND folder.created_by_user_id = actor.id
      AND folder.space_id IS NULL
      AND folder.deleted_at_ms IS NULL
    LIMIT 1
  ) AS folder_id,
  (
    SELECT folder.legacy_folder_id
    FROM folders folder
    WHERE ?3 IS NOT NULL
      AND folder.legacy_folder_id = ?3
      AND folder.organization_id = organization.id
      AND folder.created_by_user_id = actor.id
      AND folder.space_id IS NULL
      AND folder.deleted_at_ms IS NULL
    LIMIT 1
  ) AS legacy_folder_id
FROM users actor
JOIN legacy_collaboration_user_aliases_v1 actor_alias
  ON actor_alias.mapped_user_id = actor.id
JOIN organizations organization
  ON organization.status = 'active'
JOIN legacy_user_account_organization_ids_v1 organization_alias
  ON organization_alias.organization_id = organization.id
JOIN storage_integrations storage
  ON storage.organization_id = organization.id
 AND storage.provider = 'r2'
 AND storage.state = 'active'
 AND json_extract(storage.capabilities_json, '$.single_put') = 1
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND (
    (?2 IS NULL AND organization.id = actor.active_organization_id)
    OR (?2 IS NOT NULL AND organization_alias.legacy_organization_id = ?2)
  )
  AND (
    organization.owner_id = actor.id
    OR EXISTS (
      SELECT 1 FROM organization_members member
      WHERE member.organization_id = organization.id
        AND member.user_id = actor.id
        AND member.state = 'active'
    )
  )
  AND (
    ?3 IS NULL
    OR EXISTS (
      SELECT 1 FROM folders folder
      WHERE folder.legacy_folder_id = ?3
        AND folder.organization_id = organization.id
        AND folder.created_by_user_id = actor.id
        AND folder.space_id IS NULL
        AND folder.deleted_at_ms IS NULL
    )
  )
ORDER BY storage.created_at_ms, storage.id
LIMIT 2;
