INSERT INTO legacy_extension_auth_assertions_v1(
  operation_id, assertion_kind, accepted
)
SELECT ?1, 'mint_within_hourly_limit',
       CASE WHEN EXISTS (
         SELECT 1 FROM auth_api_keys k
         WHERE k.id = ?2 AND k.user_id = ?3
           AND k.key_digest = ?4 AND k.legacy_source = 'extension'
       ) THEN 1 ELSE 0 END
