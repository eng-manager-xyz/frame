INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?2,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_api_keys_v2 k
         JOIN json_each(?1) candidate
           ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = k.key_version
          AND json_extract(candidate.value, '$.digest') = k.key_digest
       ) THEN 1 ELSE 0 END
