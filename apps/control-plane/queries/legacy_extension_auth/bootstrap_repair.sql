WITH actor AS (
  SELECT id, active_organization_id
  FROM users
  WHERE id = ?1 AND status = 'active' AND deleted_at_ms IS NULL
), candidate_ids AS (
  SELECT o.id, 0 AS priority, 0 AS membership_created_at_ms
  FROM actor
  JOIN organizations o
    ON o.id = actor.active_organization_id AND o.owner_id = actor.id
   AND o.status = 'active' AND o.tombstoned_at_ms IS NULL
  UNION ALL
  SELECT o.id, 1 AS priority, 0 AS membership_created_at_ms
  FROM actor
  JOIN organizations o
    ON o.owner_id = actor.id
   AND o.status = 'active' AND o.tombstoned_at_ms IS NULL
  UNION ALL
  SELECT o.id, 2 AS priority, m.created_at_ms AS membership_created_at_ms
  FROM actor
  JOIN organization_members m
    ON m.user_id = actor.id AND m.state = 'active'
  JOIN organizations o
    ON o.id = m.organization_id
   AND o.status = 'active' AND o.tombstoned_at_ms IS NULL
), chosen AS (
  SELECT id FROM candidate_ids
  ORDER BY priority, membership_created_at_ms, id
  LIMIT 1
)
UPDATE users
SET active_organization_id = ?2,
    organization_preference_revision = organization_preference_revision + 1,
    organization_last_operation_id = ?3,
    updated_at_ms = ?4
WHERE id = ?1
  AND active_organization_id IS NOT ?2
  AND ?2 = (SELECT id FROM chosen)
