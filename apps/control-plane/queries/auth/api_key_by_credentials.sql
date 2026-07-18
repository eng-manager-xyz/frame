SELECT k.id,
       k.owner_id,
       k.tenant_id,
       k.key_version,
       k.key_digest,
       k.scopes_json,
       k.created_at_ms,
       k.expires_at_ms,
       k.revoked_at_ms,
       k.revision,
       candidate.key AS candidate_order
FROM auth_api_keys_v2 k
JOIN json_each(?1) candidate
  ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = k.key_version
 AND json_extract(candidate.value, '$.digest') = k.key_digest
ORDER BY CAST(candidate.key AS INTEGER)
LIMIT 6
