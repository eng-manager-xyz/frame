UPDATE organization_invites
SET status = 'revoked', resolved_at_ms = ?3, revision = revision + 1,
    last_operation_id = ?4
WHERE organization_id = ?1 AND invited_email_digest = ?2 AND status = 'pending'
