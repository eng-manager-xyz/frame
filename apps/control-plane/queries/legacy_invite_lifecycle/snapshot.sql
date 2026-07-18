SELECT
  actor.email AS actor_email,
  alias.mapped_invite_id,
  alias.organization_id,
  alias.invited_email,
  alias.legacy_role,
  organization.owner_id,
  owner.legacy_invite_quota AS owner_invite_quota,
  owner.legacy_stripe_subscription_id AS owner_subscription_id,
  CASE WHEN member.user_id IS NULL THEN 0 ELSE 1 END AS membership_exists,
  COALESCE(member.has_pro_seat, 0) AS membership_has_pro_seat,
  member_alias.mapped_member_id,
  member_alias.legacy_member_id,
  (
    SELECT COUNT(*)
    FROM organization_members all_member
    WHERE all_member.organization_id = alias.organization_id
      AND all_member.state = 'active'
      AND (
        all_member.has_pro_seat = 1
        OR all_member.user_id = organization.owner_id
      )
  ) AS pro_seats_used,
  (
    SELECT fallback.organization_id
    FROM organization_members fallback
    JOIN organizations fallback_org
      ON fallback_org.id = fallback.organization_id
     AND fallback_org.status = 'active'
     AND fallback_org.tombstoned_at_ms IS NULL
    WHERE fallback.user_id = actor.id
      AND fallback.organization_id <> alias.organization_id
      AND fallback.state = 'active'
    ORDER BY fallback.created_at_ms, fallback.organization_id
    LIMIT 1
  ) AS fallback_organization_id,
  (
    SELECT COUNT(*)
    FROM organization_members other_seat
    WHERE other_seat.user_id = actor.id
      AND other_seat.organization_id <> alias.organization_id
      AND other_seat.state = 'active'
      AND other_seat.has_pro_seat = 1
  ) AS other_pro_seat_count
FROM users actor
JOIN legacy_invite_lifecycle_invite_aliases_v1 alias
  ON alias.legacy_invite_id = ?2
 AND alias.decision = 'pending'
JOIN organization_invites invite
  ON invite.id = alias.mapped_invite_id
 AND invite.organization_id = alias.organization_id
JOIN organizations organization
  ON organization.id = alias.organization_id
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
LEFT JOIN users owner ON owner.id = organization.owner_id
LEFT JOIN organization_members member
  ON member.organization_id = alias.organization_id
 AND member.user_id = actor.id
 AND member.state = 'active'
LEFT JOIN legacy_invite_lifecycle_member_aliases_v1 member_alias
  ON member_alias.organization_id = alias.organization_id
 AND member_alias.user_id = actor.id
 AND member_alias.removed_at_ms IS NULL
WHERE actor.id = ?1
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
LIMIT 2;
