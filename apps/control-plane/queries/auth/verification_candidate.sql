SELECT v.id,
       v.user_id,
       v.initiator_session_id,
       v.initiator_user_id,
       v.initiator_generation,
       v.provisioning_revision,
       v.identifier_key_version,
       v.identifier_digest,
       v.secret_key_version,
       v.secret_digest,
       v.purpose,
       v.channel,
       v.attempt_count,
       v.max_attempts,
       v.created_at_ms,
       v.expires_at_ms,
       v.consumed_at_ms,
       v.state,
       v.revision,
       EXISTS (
         SELECT 1
         FROM json_each(?2) secret
         WHERE CAST(json_extract(secret.value, '$.key_version') AS INTEGER) = v.secret_key_version
           AND json_extract(secret.value, '$.digest') = v.secret_digest
       ) AS secret_matches
FROM auth_verification_challenges_v2 v
WHERE v.purpose = ?3
  AND EXISTS (
    SELECT 1
    FROM json_each(?1) identifier
    WHERE CAST(json_extract(identifier.value, '$.key_version') AS INTEGER) = v.identifier_key_version
      AND json_extract(identifier.value, '$.digest') = v.identifier_digest
  )
  AND (
    v.state = 'pending'
    OR EXISTS (
      SELECT 1
      FROM json_each(?2) exact_secret
      WHERE CAST(json_extract(exact_secret.value, '$.key_version') AS INTEGER) = v.secret_key_version
        AND json_extract(exact_secret.value, '$.digest') = v.secret_digest
    )
  )
ORDER BY secret_matches DESC, v.created_at_ms DESC, v.id DESC
LIMIT 1
