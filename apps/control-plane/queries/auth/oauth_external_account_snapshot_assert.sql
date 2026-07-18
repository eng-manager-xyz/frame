INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?4,
       CASE WHEN (
         ?3 IS NULL AND NOT EXISTS (
           SELECT 1
           FROM auth_external_accounts_v2 account
           JOIN json_each(?2) candidate
             ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = account.subject_key_version
            AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = account.subject_digest
           WHERE account.provider = ?1
         )
       ) OR (
         ?3 IS NOT NULL AND EXISTS (
           SELECT 1
           FROM auth_external_accounts_v2 account
           JOIN json_each(?2) candidate
             ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = account.subject_key_version
            AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = account.subject_digest
           WHERE account.provider = ?1
             AND account.user_id = ?3
         ) AND NOT EXISTS (
           SELECT 1
           FROM auth_external_accounts_v2 account
           JOIN json_each(?2) candidate
             ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = account.subject_key_version
            AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = account.subject_digest
           WHERE account.provider = ?1
             AND account.user_id <> ?3
         )
       ) THEN 1 ELSE 0 END
