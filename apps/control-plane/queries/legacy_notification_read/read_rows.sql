SELECT
  n.id,
  n.type AS notification_type,
  n.data_json,
  n.read_at_ms,
  n.created_at_ms,
  author.id AS author_id,
  author.display_name AS author_name,
  author.legacy_image_key AS author_image_key
FROM users actor
JOIN notifications n
  ON n.recipient_user_id = actor.id
 AND n.organization_id = actor.active_organization_id
LEFT JOIN users author
  ON n.type <> 'anon_view'
 AND json_extract(n.data_json, '$.authorId') = author.id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
ORDER BY n.read_at_ms IS NULL DESC, n.created_at_ms DESC, n.id ASC
