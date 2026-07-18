SELECT k.id AS credential_subject_id,k.user_id
FROM auth_api_keys k
JOIN users u ON u.id = k.user_id
WHERE k.key_digest = ?1
  AND k.revoked_at_ms IS NULL
  AND (k.expires_at_ms IS NULL OR k.expires_at_ms > ?2)
  AND u.status = 'active'
  AND u.deleted_at_ms IS NULL
LIMIT 1
