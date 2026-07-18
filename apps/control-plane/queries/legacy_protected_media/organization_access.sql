SELECT member.organization_id
FROM legacy_user_account_organization_ids_v1 alias
JOIN organization_members member ON member.organization_id=alias.organization_id
JOIN organizations organization ON organization.id=member.organization_id
JOIN users actor ON actor.id=member.user_id
WHERE alias.legacy_organization_id=?1 AND member.user_id=?2
  AND member.state='active'
  AND organization.status='active'
  AND actor.status='active' AND actor.deleted_at_ms IS NULL
LIMIT 2;
