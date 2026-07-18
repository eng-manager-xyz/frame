SELECT n.type AS notification_type, COUNT(*) AS notification_count
FROM users actor
JOIN notifications n
  ON n.recipient_user_id = actor.id
 AND n.organization_id = actor.active_organization_id
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
GROUP BY n.type
ORDER BY n.type
