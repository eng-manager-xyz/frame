SELECT
  actor.id AS mapped_user_id,
  actor_alias.legacy_user_id AS legacy_user_id,
  actor.display_name AS display_name,
  actor.email AS email,
  actor.legacy_image_key AS image_key,
  actor.active_organization_id AS active_organization_id,
  active_alias.legacy_organization_id AS active_legacy_organization_id
FROM users actor
LEFT JOIN legacy_collaboration_user_aliases_v1 actor_alias
  ON actor_alias.mapped_user_id = actor.id
LEFT JOIN legacy_user_account_organization_ids_v1 active_alias
  ON active_alias.organization_id = actor.active_organization_id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
LIMIT 2;
