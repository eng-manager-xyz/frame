INSERT INTO legacy_mobile_upload_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'authority', 1, COUNT(*)
FROM users actor
JOIN legacy_collaboration_user_aliases_v1 actor_alias
  ON actor_alias.mapped_user_id = actor.id AND actor_alias.legacy_user_id = ?3
JOIN organizations organization
  ON organization.id = ?4 AND organization.status = 'active'
JOIN storage_integrations storage
  ON storage.id = ?5 AND storage.organization_id = organization.id
 AND storage.provider = 'r2' AND storage.state = 'active'
 AND json_extract(storage.capabilities_json, '$.single_put') = 1
WHERE actor.id = ?2
  AND actor.status = 'active' AND actor.deleted_at_ms IS NULL
  AND (
    organization.owner_id = actor.id
    OR EXISTS (
      SELECT 1 FROM organization_members member
      WHERE member.organization_id = organization.id
        AND member.user_id = actor.id AND member.state = 'active'
    )
  )
  AND (
    ?6 IS NULL
    OR EXISTS (
      SELECT 1 FROM folders folder
      WHERE folder.id = ?6 AND folder.organization_id = organization.id
        AND folder.created_by_user_id = actor.id
        AND folder.space_id IS NULL AND folder.deleted_at_ms IS NULL
    )
  );
