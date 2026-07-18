DELETE FROM auth_pending_verifications_v2
WHERE purpose = ?1
  AND EXISTS (
    SELECT 1
    FROM json_each(?2) candidate
    WHERE CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = active_identifier_key_version
      AND json_extract(candidate.value, '$.digest') = active_identifier_digest
  )
