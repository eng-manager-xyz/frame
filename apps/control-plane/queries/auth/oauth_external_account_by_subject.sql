WITH candidates AS (
  SELECT CAST(key AS INTEGER) candidate_order,
         CAST(json_extract(value, '$.key_version') AS INTEGER) key_version,
         CAST(json_extract(value, '$.digest') AS TEXT) digest
  FROM json_each(?2)
)
SELECT account.user_id,
       account.subject_key_version,
       account.subject_digest,
       candidates.candidate_order
FROM candidates
JOIN auth_external_accounts_v2 account
  ON account.subject_key_version = candidates.key_version
 AND account.subject_digest = candidates.digest
WHERE account.provider = ?1
ORDER BY candidates.candidate_order
LIMIT 5
