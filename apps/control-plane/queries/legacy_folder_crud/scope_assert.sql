INSERT INTO legacy_folder_crud_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'scope', 1,
  (
    SELECT COUNT(*)
    FROM organizations o
    LEFT JOIN spaces s
      ON ?4 = 'space'
     AND s.id = ?5
     AND s.organization_id = o.id
     AND s.deleted_at_ms IS NULL
    LEFT JOIN space_members sm
      ON sm.space_id = s.id
     AND sm.user_id = ?3
     AND sm.state = 'active'
    WHERE o.id = ?2
      AND o.status = 'active'
      AND o.tombstoned_at_ms IS NULL
      AND (
        (?4 = 'personal' AND ?5 IS NULL AND ?6 = -1 AND ?7 = -1)
        OR (?4 = 'organization' AND ?5 = o.id AND ?6 = -1 AND ?7 = -1)
        OR (
          ?4 = 'space' AND s.id IS NOT NULL
          AND s.revision = ?6 AND s.authority_version = ?7
          AND s.created_by_user_id = ?8
          AND COALESCE(sm.role, '') = ?9
          AND COALESCE(sm.revision, -1) = ?10
        )
      )
  )
)
