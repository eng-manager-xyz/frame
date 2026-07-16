INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?2,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_session_credentials_v2 c
         JOIN json_each(?1) candidate
           ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = c.key_version
          AND json_extract(candidate.value, '$.digest') = c.digest
       ) THEN 1 ELSE 0 END
