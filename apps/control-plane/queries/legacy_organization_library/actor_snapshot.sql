SELECT id, organization_preference_revision
FROM users
WHERE id = ?1 AND status = 'active' AND deleted_at_ms IS NULL
LIMIT 2
