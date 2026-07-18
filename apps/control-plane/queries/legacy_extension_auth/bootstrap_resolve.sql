WITH actor AS (
  SELECT id, active_organization_id
  FROM users
  WHERE id = ?1 AND status = 'active' AND deleted_at_ms IS NULL
), candidates AS (
  SELECT o.id, COALESCE(o.legacy_user_account_name, o.name) AS name,
         owner.legacy_stripe_subscription_status AS stripe_subscription_status,
         owner.legacy_third_party_stripe_subscription_id AS third_party_stripe_subscription_id,
         actor.active_organization_id,
         0 AS priority, 0 AS membership_created_at_ms
  FROM actor
  JOIN organizations o
    ON o.id = actor.active_organization_id AND o.owner_id = actor.id
   AND o.status = 'active' AND o.tombstoned_at_ms IS NULL
  JOIN users owner
    ON owner.id = o.owner_id AND owner.status = 'active' AND owner.deleted_at_ms IS NULL

  UNION ALL

  SELECT o.id, COALESCE(o.legacy_user_account_name, o.name) AS name,
         owner.legacy_stripe_subscription_status AS stripe_subscription_status,
         owner.legacy_third_party_stripe_subscription_id AS third_party_stripe_subscription_id,
         actor.active_organization_id,
         1 AS priority, 0 AS membership_created_at_ms
  FROM actor
  JOIN organizations o
    ON o.owner_id = actor.id
   AND o.status = 'active' AND o.tombstoned_at_ms IS NULL
  JOIN users owner
    ON owner.id = o.owner_id AND owner.status = 'active' AND owner.deleted_at_ms IS NULL

  UNION ALL

  SELECT o.id, COALESCE(o.legacy_user_account_name, o.name) AS name,
         owner.legacy_stripe_subscription_status AS stripe_subscription_status,
         owner.legacy_third_party_stripe_subscription_id AS third_party_stripe_subscription_id,
         actor.active_organization_id,
         2 AS priority, m.created_at_ms AS membership_created_at_ms
  FROM actor
  JOIN organization_members m
    ON m.user_id = actor.id AND m.state = 'active'
  JOIN organizations o
    ON o.id = m.organization_id
   AND o.status = 'active' AND o.tombstoned_at_ms IS NULL
  JOIN users owner
    ON owner.id = o.owner_id AND owner.status = 'active' AND owner.deleted_at_ms IS NULL
)
SELECT id, name, stripe_subscription_status, third_party_stripe_subscription_id,
       active_organization_id
FROM candidates
ORDER BY priority, membership_created_at_ms, id
LIMIT 1
