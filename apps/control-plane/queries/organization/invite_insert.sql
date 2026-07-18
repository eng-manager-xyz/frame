INSERT INTO organization_invites(
  id, organization_id, invited_email_key_version, invited_email_digest,
  invited_by_user_id, role,
  status, token_digest, created_at_ms, expires_at_ms, resolved_at_ms,
  revision, accepted_by_user_id, last_operation_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, NULL, 0, NULL, ?10)
