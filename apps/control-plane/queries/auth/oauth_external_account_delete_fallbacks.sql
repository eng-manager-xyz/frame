DELETE FROM auth_external_accounts_v2
WHERE provider = ?1
  AND user_id = ?2
  AND NOT (subject_key_version = ?3 AND subject_digest = ?4)
  AND EXISTS (
    SELECT 1
    FROM json_each(?5) candidate
    WHERE CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = subject_key_version
      AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = subject_digest
  )
