WITH candidates AS (
  SELECT CAST(key AS INTEGER) candidate_order,
         CAST(json_extract(value, '$.key_version') AS INTEGER) key_version,
         CAST(json_extract(value, '$.digest') AS TEXT) digest
  FROM json_each(?1)
)
SELECT flow.id,
       flow.provider,
       flow.purpose,
       flow.initiator_session_id,
       flow.initiator_user_id,
       flow.initiator_generation,
       flow.state_key_version,
       flow.state_digest,
       flow.pkce_key_version,
       flow.pkce_digest,
       flow.redirect_key_version,
       flow.redirect_digest,
       flow.audience_key_version,
       flow.audience_digest,
       flow.created_at_ms,
       flow.expires_at_ms,
       flow.consumed_at_ms,
       flow.revoked,
       flow.revision,
       flow.last_operation_id,
       candidates.candidate_order
FROM candidates
JOIN auth_oauth_flows_v2 flow
  ON flow.state_key_version = candidates.key_version
 AND flow.state_digest = candidates.digest
ORDER BY candidates.candidate_order
LIMIT 2
