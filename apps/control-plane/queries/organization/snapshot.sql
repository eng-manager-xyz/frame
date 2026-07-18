SELECT o.id,
       o.owner_id,
       o.name,
       o.status,
       o.settings_json,
       o.created_at_ms,
       o.updated_at_ms,
       o.tombstoned_at_ms,
       o.retention_until_ms,
       o.revision,
       o.authority_version,
       m.role AS actor_role,
       m.state AS actor_state,
       m.has_pro_seat,
       m.created_at_ms AS member_created_at_ms,
       m.updated_at_ms AS member_updated_at_ms,
       m.revision AS member_revision,
       m.authority_version AS member_authority_version,
       u.default_organization_id,
       u.active_organization_id,
       u.organization_preference_revision
FROM organizations o
JOIN organization_members m
  ON m.organization_id = o.id AND m.user_id = ?2
JOIN users u ON u.id = m.user_id AND u.status = 'active'
JOIN auth_identities_v2 i
  ON i.user_id = u.id AND i.identity_revision = ?3 AND i.session_version = ?4
WHERE o.id = ?1 AND m.state = 'active' AND o.status <> 'deleted'
LIMIT 1
