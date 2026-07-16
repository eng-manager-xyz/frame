INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?7,
       CASE WHEN EXISTS (
         SELECT 1
         FROM auth_external_accounts_v2 account
         WHERE account.provider = ?1
           AND account.subject_key_version = ?2
           AND account.subject_digest = ?3
           AND account.user_id = ?4
           AND account.last_operation_id = ?5
       ) AND NOT EXISTS (
         SELECT 1
         FROM auth_external_accounts_v2 account
         JOIN json_each(?6) candidate
           ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = account.subject_key_version
          AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = account.subject_digest
         WHERE account.provider = ?1
           AND NOT (
             account.subject_key_version = ?2
             AND account.subject_digest = ?3
             AND account.user_id = ?4
           )
       ) THEN 1 ELSE 0 END
