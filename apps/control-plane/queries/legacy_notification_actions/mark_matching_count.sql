SELECT COUNT(*) AS matching_count
FROM notifications n
WHERE n.organization_id = ?1
  AND n.recipient_user_id = ?2
  AND (?3 IS NULL OR n.id = ?3)
LIMIT 1
