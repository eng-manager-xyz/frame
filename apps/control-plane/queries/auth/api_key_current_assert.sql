INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?6,
       CASE WHEN EXISTS (
         SELECT 1 FROM auth_api_keys_v2
         WHERE id = ?1
           AND revision = ?2
           AND key_version = ?3
           AND key_digest = ?4
           AND ((?5 IS NULL AND revoked_at_ms IS NULL) OR revoked_at_ms = ?5)
       ) THEN 1 ELSE 0 END
