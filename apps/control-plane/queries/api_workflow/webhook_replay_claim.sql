INSERT OR IGNORE INTO api_webhook_replay_claims_v1(
  replay_digest, claimed_at_ms, expires_at_ms
) VALUES (?1, ?2, ?3)
RETURNING replay_digest;
