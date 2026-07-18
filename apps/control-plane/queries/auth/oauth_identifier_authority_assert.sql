INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?4,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_identifier_digests_v2 identifier
         JOIN json_each(?1) candidate
           ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = identifier.key_version
          AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = identifier.digest
         WHERE identifier.user_id <> ?2
       ) AND (
         ?3 = 1 OR EXISTS (
           SELECT 1
           FROM auth_identifier_digests_v2 identifier
           JOIN json_each(?1) candidate
             ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = identifier.key_version
            AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = identifier.digest
           WHERE identifier.user_id = ?2
         )
       ) THEN 1 ELSE 0 END
