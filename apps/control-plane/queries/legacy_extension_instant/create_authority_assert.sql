INSERT INTO legacy_extension_instant_assertions_v1(operation_id, assertion_kind, accepted)
SELECT ?1, 'create_authority', CASE WHEN COUNT(*) = 1 THEN 1 ELSE 0 END
FROM (
  SELECT storage.id
  FROM users actor
  JOIN organizations organization
    ON organization.id = ?3 AND organization.status = 'active'
  JOIN storage_integrations storage
    ON storage.id = ?5
   AND storage.organization_id = organization.id
   AND storage.provider = 'r2' AND storage.state = 'active'
  WHERE actor.id = ?2
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
      ?4 IS NULL
      OR EXISTS (
        SELECT 1 FROM folders folder
        WHERE folder.id = ?4
          AND folder.organization_id = organization.id
          AND folder.deleted_at_ms IS NULL
      )
    )
  LIMIT 2
);
