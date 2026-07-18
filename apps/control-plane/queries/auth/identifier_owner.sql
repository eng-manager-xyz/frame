SELECT d.user_id,
       d.key_version,
       d.digest
FROM auth_identifier_digests_v2 d
JOIN auth_identities_v2 i ON i.user_id = d.user_id
JOIN json_each(?1) candidate
  ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = d.key_version
 AND json_extract(candidate.value, '$.digest') = d.digest
ORDER BY CAST(candidate.key AS INTEGER)
LIMIT 2
