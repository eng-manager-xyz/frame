UPDATE organization_invites
SET status = 'accepted',
    accepted_by_user_id = ?5,
    resolved_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    revision = revision + 1,
    last_operation_id = ?6
WHERE id = ?1
  AND organization_id = ?2
  AND status = 'pending'
  AND revision = ?3
  AND token_digest = ?4
  AND expires_at_ms > CAST(strftime('%s', 'now') AS INTEGER) * 1000
  AND EXISTS (
    SELECT 1
    FROM users u
    JOIN auth_identities_v2 i ON i.user_id = u.id
    JOIN auth_identifier_digests_v2 identifier
      ON identifier.user_id = u.id
     AND identifier.key_version = organization_invites.invited_email_key_version
     AND identifier.digest = organization_invites.invited_email_digest
    WHERE u.id = ?5 AND u.status = 'active'
  )
