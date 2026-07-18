UPDATE legacy_developer_apps_v1
SET deleted_at_ms = ?6, updated_at_ms = ?6, revision = revision + 1,
    authority_version = authority_version + 1, last_operation_id = ?1
WHERE id = ?2 AND owner_id = ?3 AND deleted_at_ms IS NULL
  AND revision = ?4 AND authority_version = ?5
