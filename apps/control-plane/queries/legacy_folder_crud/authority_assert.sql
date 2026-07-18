INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'authority', 1,
  (
    SELECT COUNT(*)
    FROM users u
    JOIN organizations o
      ON o.id = u.active_organization_id
     AND o.status = 'active'
     AND o.tombstoned_at_ms IS NULL
    JOIN organization_members m
      ON m.organization_id = o.id
     AND m.user_id = u.id
     AND m.state = 'active'
    JOIN organization_members owner_membership
      ON owner_membership.organization_id = o.id
     AND owner_membership.user_id = o.owner_id
     AND owner_membership.role = 'owner'
     AND owner_membership.state = 'active'
    WHERE u.id = ?2
      AND u.status = 'active'
      AND u.deleted_at_ms IS NULL
      AND u.active_organization_id = ?3
      AND u.organization_preference_revision = ?4
      AND o.owner_id = ?5
      AND o.revision = ?6
      AND o.authority_version = ?7
      AND m.role = ?8
      AND m.revision = ?9
      AND m.authority_version = ?10
      AND owner_membership.has_pro_seat = ?11
      AND owner_membership.revision = ?12
      AND owner_membership.authority_version = ?13
  )
)
