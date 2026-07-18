WITH candidate AS (
  SELECT
    space.id AS collection_id,
    space.organization_id,
    space.legacy_password_hash AS password_hash,
    space.legacy_password_revision AS password_revision,
    'space' AS collection_kind,
    1 AS source_priority
  FROM spaces space
  JOIN organizations organization
    ON organization.id = space.organization_id
   AND organization.status = 'active'
   AND organization.tombstoned_at_ms IS NULL
  WHERE space.id = ?1 AND space.is_public = 1 AND space.deleted_at_ms IS NULL
  UNION ALL
  SELECT
    folder.id AS collection_id,
    folder.organization_id,
    parent_space.legacy_password_hash AS password_hash,
    parent_space.legacy_password_revision AS password_revision,
    'folder' AS collection_kind,
    0 AS source_priority
  FROM folders folder
  JOIN organizations organization
    ON organization.id = folder.organization_id
   AND organization.status = 'active'
   AND organization.tombstoned_at_ms IS NULL
  LEFT JOIN spaces parent_space
    ON parent_space.id = folder.space_id
   AND parent_space.organization_id = folder.organization_id
   AND parent_space.deleted_at_ms IS NULL
  WHERE folder.id = ?1 AND folder.is_public = 1 AND folder.deleted_at_ms IS NULL
)
SELECT collection_id, organization_id, password_hash,
       COALESCE(password_revision, 0) AS password_revision, collection_kind
FROM candidate
ORDER BY source_priority
LIMIT 1
