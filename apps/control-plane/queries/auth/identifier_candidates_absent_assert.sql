INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?2,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_identifier_digests_v2 i
         JOIN json_each(?1) candidate
           ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = i.key_version
          AND json_extract(candidate.value, '$.digest') = i.digest
       ) THEN 1 ELSE 0 END
