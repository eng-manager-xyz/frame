UPDATE auth_verification_challenges_v2
SET state = 'revoked',
    revision = revision + 1,
    last_operation_id = ?3
WHERE purpose = ?1
  AND state = 'pending'
  AND EXISTS (
    SELECT 1
    FROM json_each(?2) candidate
    WHERE CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = identifier_key_version
      AND json_extract(candidate.value, '$.digest') = identifier_digest
  )
