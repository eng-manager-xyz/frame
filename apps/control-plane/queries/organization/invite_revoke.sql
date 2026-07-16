UPDATE organization_invites
SET status = 'revoked',
    resolved_at_ms = ?4,
    revision = revision + 1,
    last_operation_id = ?5
WHERE id = ?1
  AND organization_id = ?2
  AND status = 'pending'
  AND revision = ?3
  AND expires_at_ms > ?4
