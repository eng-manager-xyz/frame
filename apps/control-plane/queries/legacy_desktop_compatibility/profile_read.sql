SELECT
  user.display_name AS first_name,
  user.legacy_last_name AS last_name,
  user.email,
  COALESCE(user_alias.image_url, user.legacy_image_key) AS image_url
FROM users user
LEFT JOIN legacy_collaboration_user_aliases_v1 user_alias
  ON user_alias.mapped_user_id = user.id
WHERE user.id = ?1
  AND user.status = 'active'
  AND user.deleted_at_ms IS NULL
LIMIT 2;
