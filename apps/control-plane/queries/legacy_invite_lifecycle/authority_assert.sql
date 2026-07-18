INSERT INTO legacy_invite_lifecycle_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'live_invite_and_casefolded_email', 1, COUNT(*)
FROM users actor
JOIN legacy_invite_lifecycle_invite_aliases_v1 alias
  ON alias.legacy_invite_id = ?4
 AND alias.mapped_invite_id = ?5
 AND alias.organization_id = ?3
 AND alias.invited_email = ?6
 AND alias.legacy_role = ?7
 AND alias.decision = 'pending'
JOIN organization_invites invite
  ON invite.id = alias.mapped_invite_id
 AND invite.organization_id = alias.organization_id
JOIN organizations organization
  ON organization.id = alias.organization_id
 AND organization.status = 'active'
 AND organization.tombstoned_at_ms IS NULL
LEFT JOIN users owner ON owner.id = organization.owner_id
WHERE actor.id = ?2
  AND actor.email = ?8
  AND actor.status = 'active'
  AND actor.deleted_at_ms IS NULL
  AND alias.invited_email COLLATE BINARY = ?6 COLLATE BINARY
  AND organization.owner_id = ?9
  AND owner.legacy_invite_quota IS ?10
  AND owner.legacy_stripe_subscription_id IS ?11
  AND (
    SELECT COUNT(*)
    FROM organization_members member
    WHERE member.organization_id = alias.organization_id
      AND member.user_id = actor.id
      AND member.state = 'active'
  ) = ?12
  AND COALESCE((
    SELECT member.has_pro_seat
    FROM organization_members member
    WHERE member.organization_id = alias.organization_id
      AND member.user_id = actor.id
      AND member.state = 'active'
    LIMIT 1
  ), 0) = ?13
  AND (
    SELECT member_alias.mapped_member_id
    FROM legacy_invite_lifecycle_member_aliases_v1 member_alias
    WHERE member_alias.organization_id = alias.organization_id
      AND member_alias.user_id = actor.id
      AND member_alias.removed_at_ms IS NULL
    LIMIT 1
  ) IS ?14
  AND (
    SELECT member_alias.legacy_member_id
    FROM legacy_invite_lifecycle_member_aliases_v1 member_alias
    WHERE member_alias.organization_id = alias.organization_id
      AND member_alias.user_id = actor.id
      AND member_alias.removed_at_ms IS NULL
    LIMIT 1
  ) IS ?15
  AND (
    SELECT COUNT(*)
    FROM organization_members all_member
    WHERE all_member.organization_id = alias.organization_id
      AND all_member.state = 'active'
      AND (
        all_member.has_pro_seat = 1
        OR all_member.user_id = organization.owner_id
      )
  ) = ?16
  AND (
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
  ) IS ?17
  AND (
    SELECT COUNT(*)
    FROM organization_members other_seat
    WHERE other_seat.user_id = actor.id
      AND other_seat.organization_id <> alias.organization_id
      AND other_seat.state = 'active'
      AND other_seat.has_pro_seat = 1
  ) = ?18;
