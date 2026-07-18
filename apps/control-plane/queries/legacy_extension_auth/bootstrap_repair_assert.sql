INSERT INTO legacy_extension_auth_assertions_v1(
  operation_id, assertion_kind, accepted
)
SELECT ?1, 'bootstrap_selection_repaired',
       CASE WHEN EXISTS (
         SELECT 1 FROM users u
         WHERE u.id = ?2 AND u.active_organization_id = ?3
           AND u.status = 'active' AND u.deleted_at_ms IS NULL
       ) THEN 1 ELSE 0 END
