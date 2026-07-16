SELECT id, organization_id, invited_by_user_id, accepted_by_user_id,
       role, status, created_at_ms, expires_at_ms, resolved_at_ms, revision
FROM organization_invites
WHERE organization_id = ?1
  AND (?2 IS NULL OR id > ?2)
ORDER BY id
LIMIT ?3
