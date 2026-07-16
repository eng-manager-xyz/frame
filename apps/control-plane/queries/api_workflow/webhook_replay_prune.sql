DELETE FROM api_webhook_replay_claims_v1
 WHERE replay_digest IN (
   SELECT replay_digest
     FROM api_webhook_replay_claims_v1
    WHERE expires_at_ms < ?1
    ORDER BY expires_at_ms, replay_digest
    LIMIT ?2
 )
RETURNING replay_digest;
