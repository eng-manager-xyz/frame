INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'product_effect',
  1,
  CASE WHEN
    -- The optional target folder and its real active space are unchanged.
    (
      ?5 = '' OR EXISTS (
        SELECT 1
        FROM folders f
        LEFT JOIN spaces s
          ON s.id = f.space_id
         AND s.organization_id = f.organization_id
         AND s.deleted_at_ms IS NULL
        LEFT JOIN space_members sm
          ON sm.space_id = s.id
         AND sm.user_id = ?3
         AND sm.state = 'active'
        WHERE f.id = ?5
          AND f.organization_id = ?2
          AND f.deleted_at_ms IS NULL
          AND f.revision = ?6
          AND f.tree_revision = ?7
          AND f.space_id IS ?8
          AND f.parent_id IS ?21
          AND f.created_by_user_id = ?9
          AND COALESCE(s.revision, -1) = ?10
          AND COALESCE(s.authority_version, -1) = ?11
          AND (f.space_id IS NULL OR s.id IS NOT NULL)
          AND COALESCE(sm.role, '') = ?12
          AND COALESCE(sm.revision, -1) = ?13
          AND (
            f.parent_id IS NULL OR EXISTS (
              SELECT 1
              FROM folders parent
              LEFT JOIN spaces parent_space
                ON parent_space.id = parent.space_id
               AND parent_space.organization_id = parent.organization_id
               AND parent_space.deleted_at_ms IS NULL
              WHERE parent.id = f.parent_id
                AND parent.organization_id = f.organization_id
                AND parent.deleted_at_ms IS NULL
                AND parent.space_id IS f.space_id
                AND (parent.space_id IS NULL OR parent_space.id IS NOT NULL)
            )
          )
      )
    )
    -- The original folder used to derive move invalidations is also a live,
    -- exact tenant graph fact. List actions carry ''.
    AND (
      ?22 = '' OR EXISTS (
        SELECT 1
        FROM folders original
        LEFT JOIN spaces original_space
          ON original_space.id = original.space_id
         AND original_space.organization_id = original.organization_id
         AND original_space.deleted_at_ms IS NULL
        WHERE original.id = ?22
          AND original.organization_id = ?2
          AND original.deleted_at_ms IS NULL
          AND original.revision = ?23
          AND original.tree_revision = ?24
          AND original.space_id IS ?25
          AND original.parent_id IS ?26
          AND COALESCE(original_space.revision, -1) = ?27
          AND COALESCE(original_space.authority_version, -1) = ?28
          AND (original.space_id IS NULL OR original_space.id IS NOT NULL)
          AND (
            original.parent_id IS NULL OR EXISTS (
              SELECT 1
              FROM folders parent
              LEFT JOIN spaces parent_space
                ON parent_space.id = parent.space_id
               AND parent_space.organization_id = parent.organization_id
               AND parent_space.deleted_at_ms IS NULL
              WHERE parent.id = original.parent_id
                AND parent.organization_id = original.organization_id
                AND parent.deleted_at_ms IS NULL
                AND parent.space_id IS original.space_id
                AND (parent.space_id IS NULL OR parent_space.id IS NOT NULL)
            )
          )
      )
    )
    -- A real-space context is exact and active; other contexts carry ''.
    AND (
      ?14 = '' OR EXISTS (
        SELECT 1
        FROM spaces s
        LEFT JOIN space_members sm
          ON sm.space_id = s.id
         AND sm.user_id = ?3
         AND sm.state = 'active'
        WHERE s.id = ?14
          AND s.organization_id = ?2
          AND s.deleted_at_ms IS NULL
          AND s.revision = ?15
          AND s.authority_version = ?16
          AND COALESCE(sm.role, '') = ?17
          AND COALESCE(sm.revision, -1) = ?18
      )
    )
    -- Every requested video still has the exact tenant-bound snapshot.
    AND NOT EXISTS (
      SELECT 1
      FROM json_each(?19) expected
      LEFT JOIN videos v
        ON v.id = json_extract(expected.value, '$.id')
       AND v.organization_id = ?2
       AND v.deleted_at_ms IS NULL
      WHERE v.id IS NULL
         OR v.owner_id <> json_extract(expected.value, '$.owner_id')
         OR v.folder_id IS NOT json_extract(expected.value, '$.folder_id')
         OR v.revision <> CAST(json_extract(expected.value, '$.revision') AS INTEGER)
    )
    -- The selected scoped membership snapshot is also exact. It is empty for
    -- direct moves, one space_videos snapshot per ID for a real space, and one
    -- logical current shared_videos snapshot per ID for organization library.
    AND (
      ?4 = 'direct'
      OR (?4 = 'space' AND NOT EXISTS (
        SELECT 1
        FROM json_each(?20) expected
        LEFT JOIN space_videos sv
          ON sv.space_id = ?14
         AND sv.video_id = json_extract(expected.value, '$.id')
        WHERE (sv.video_id IS NOT NULL) <> CAST(json_extract(expected.value, '$.present') AS INTEGER)
           OR COALESCE(sv.folder_id, '') <> COALESCE(json_extract(expected.value, '$.folder_id'), '')
           OR COALESCE(sv.revision, -1) <> CAST(json_extract(expected.value, '$.revision') AS INTEGER)
      ))
      OR (?4 = 'organization' AND NOT EXISTS (
        SELECT 1
        FROM json_each(?20) expected
        WHERE (
          SELECT COUNT(*) FROM shared_videos current
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
        ) <> CAST(json_extract(expected.value, '$.active_count') AS INTEGER)
        OR COALESCE((
          SELECT current.id FROM shared_videos current
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.active_id'), '')
        OR COALESCE((
          SELECT current.folder_id FROM shared_videos current
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.active_folder_id'), '')
        OR COALESCE((
          SELECT current.revision FROM shared_videos current
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), -1) <> CAST(json_extract(expected.value, '$.active_revision') AS INTEGER)
        OR COALESCE((
          SELECT dormant.id FROM shared_videos dormant
          WHERE dormant.organization_id = ?2
            AND dormant.video_id = json_extract(expected.value, '$.id')
            AND dormant.revoked_at_ms IS NOT NULL
            AND dormant.folder_id IS json_extract(expected.value, '$.desired_folder_id')
          ORDER BY dormant.revoked_at_ms DESC, dormant.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.dormant_id'), '')
        OR COALESCE((
          SELECT dormant.revision FROM shared_videos dormant
          WHERE dormant.organization_id = ?2
            AND dormant.video_id = json_extract(expected.value, '$.id')
            AND dormant.revoked_at_ms IS NOT NULL
            AND dormant.folder_id IS json_extract(expected.value, '$.desired_folder_id')
          ORDER BY dormant.revoked_at_ms DESC, dormant.id LIMIT 1
        ), -1) <> CAST(json_extract(expected.value, '$.dormant_revision') AS INTEGER)
      ))
    )
    -- Add/remove preserve Cap's actor-owned filter but make it all-or-nothing:
    -- every canonical video must be owned even for an organization owner or
    -- space manager. Move may target a tenant video without actor ownership
    -- only when the actor manages the selected organization/space/folder.
    AND (
      ?29 = 'move' OR NOT EXISTS (
        SELECT 1
        FROM json_each(?19) expected
        WHERE json_extract(expected.value, '$.owner_id') <> ?3
      )
    )
    AND EXISTS (
      SELECT 1
      FROM organization_members om
      WHERE om.organization_id = ?2
        AND om.user_id = ?3
        AND om.state = 'active'
        AND (
          om.role IN ('owner', 'admin')
          OR (
            om.role = 'member'
            AND (
              (?4 IN ('direct', 'organization') AND (
                NOT EXISTS (
                  SELECT 1
                  FROM json_each(?19) expected
                  WHERE json_extract(expected.value, '$.owner_id') <> ?3
                )
                OR (?29 = 'move' AND ?5 <> '' AND (
                  ?12 = 'manager' OR (?8 IS NULL AND ?9 = ?3)
                ))
              ))
              OR (?4 = 'space' AND (
                ?17 = 'manager'
                OR (?17 = 'contributor' AND NOT EXISTS (
                  SELECT 1
                  FROM json_each(?19) expected
                  WHERE json_extract(expected.value, '$.owner_id') <> ?3
                ))
              ))
            )
            AND (
              ?5 = '' OR ?12 = 'manager'
              OR (?12 = 'contributor' AND ?9 = ?3)
              OR (?8 IS NULL AND ?9 = ?3)
            )
          )
        )
    )
  THEN 1 ELSE 0 END
)
