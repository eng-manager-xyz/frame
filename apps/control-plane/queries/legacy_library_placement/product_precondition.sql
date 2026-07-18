INSERT INTO authenticated_web_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1,
  'product_effect',
  1,
  CASE WHEN
    -- The selected real space and manager membership remain exact.  An
    -- organization-root scope carries an empty space id and -1 sentinels.
    (
      (?5 = 'organization' AND ?6 = '')
      OR (?5 = 'space' AND EXISTS (
        SELECT 1
        FROM spaces space
        LEFT JOIN space_members membership
          ON membership.space_id = space.id
         AND membership.user_id = ?3
         AND membership.state = 'active'
        WHERE space.id = ?6
          AND space.organization_id = ?2
          AND space.deleted_at_ms IS NULL
          AND space.revision = ?7
          AND space.authority_version = ?8
          AND COALESCE(membership.role, '') = ?9
          AND COALESCE(membership.revision, -1) = ?10
      ))
    )
    -- Every tenant video snapshot is exact.  Missing videos are represented
    -- by present=0 and are permitted only by remove-organization.
    AND NOT EXISTS (
      SELECT 1
      FROM json_each(?11) expected
      LEFT JOIN videos video
        ON video.id = json_extract(expected.value, '$.id')
       AND video.organization_id = ?2
       AND video.deleted_at_ms IS NULL
      WHERE (video.id IS NOT NULL) <> CAST(json_extract(expected.value, '$.present') AS INTEGER)
         OR COALESCE(video.owner_id, '') <> COALESCE(json_extract(expected.value, '$.owner_id'), '')
         OR COALESCE(video.folder_id, '') <> COALESCE(json_extract(expected.value, '$.folder_id'), '')
         OR COALESCE(video.revision, -1) <> CAST(json_extract(expected.value, '$.revision') AS INTEGER)
         OR CAST(json_extract(expected.value, '$.folder_in_tenant') AS INTEGER) <> CASE
           WHEN video.folder_id IS NOT NULL AND EXISTS (
             SELECT 1
             FROM folders folder
             LEFT JOIN spaces folder_space
               ON folder_space.id = folder.space_id
              AND folder_space.organization_id = folder.organization_id
              AND folder_space.deleted_at_ms IS NULL
             WHERE folder.id = video.folder_id
               AND folder.organization_id = ?2
               AND folder.deleted_at_ms IS NULL
               AND (folder.space_id IS NULL OR folder_space.id IS NOT NULL)
           ) THEN 1 ELSE 0 END
    )
    -- The ownership asymmetry is deliberate: removing organization shares may
    -- match any tenant-scoped share; the other three actions are all-or-nothing
    -- and require every canonical video to be actor-owned.
    AND (
      ?4 = 'remove_organization'
      OR NOT EXISTS (
        SELECT 1 FROM json_each(?11) expected
        WHERE CAST(json_extract(expected.value, '$.present') AS INTEGER) <> 1
           OR json_extract(expected.value, '$.owner_id') <> ?3
      )
    )
    -- The selected normalized membership snapshot is exact.
    AND (
      (?5 = 'organization' AND NOT EXISTS (
        SELECT 1
        FROM json_each(?12) expected
        WHERE (
          SELECT COUNT(*)
          FROM shared_videos current
          JOIN videos tenant_video
            ON tenant_video.id = current.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
        ) <> CAST(json_extract(expected.value, '$.active_count') AS INTEGER)
        OR COALESCE((
          SELECT current.id
          FROM shared_videos current
          JOIN videos tenant_video
            ON tenant_video.id = current.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.active_id'), '')
        OR COALESCE((
          SELECT current.folder_id
          FROM shared_videos current
          JOIN videos tenant_video
            ON tenant_video.id = current.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.active_folder_id'), '')
        OR COALESCE((
          SELECT current.sharing_mode
          FROM shared_videos current
          JOIN videos tenant_video
            ON tenant_video.id = current.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.active_sharing_mode'), '')
        OR COALESCE((
          SELECT current.revision
          FROM shared_videos current
          JOIN videos tenant_video
            ON tenant_video.id = current.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE current.organization_id = ?2
            AND current.video_id = json_extract(expected.value, '$.id')
            AND current.revoked_at_ms IS NULL
          ORDER BY current.id LIMIT 1
        ), -1) <> CAST(json_extract(expected.value, '$.active_revision') AS INTEGER)
        OR COALESCE((
          SELECT dormant.id
          FROM shared_videos dormant
          JOIN videos tenant_video
            ON tenant_video.id = dormant.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE dormant.organization_id = ?2
            AND dormant.video_id = json_extract(expected.value, '$.id')
            AND dormant.revoked_at_ms IS NOT NULL
            AND dormant.folder_id IS NULL
          ORDER BY dormant.revoked_at_ms DESC, dormant.id LIMIT 1
        ), '') <> COALESCE(json_extract(expected.value, '$.dormant_root_id'), '')
        OR COALESCE((
          SELECT dormant.revision
          FROM shared_videos dormant
          JOIN videos tenant_video
            ON tenant_video.id = dormant.video_id
           AND tenant_video.organization_id = ?2
           AND tenant_video.deleted_at_ms IS NULL
          WHERE dormant.organization_id = ?2
            AND dormant.video_id = json_extract(expected.value, '$.id')
            AND dormant.revoked_at_ms IS NOT NULL
            AND dormant.folder_id IS NULL
          ORDER BY dormant.revoked_at_ms DESC, dormant.id LIMIT 1
        ), -1) <> CAST(json_extract(expected.value, '$.dormant_root_revision') AS INTEGER)
      ))
      OR (?5 = 'space' AND NOT EXISTS (
        SELECT 1
        FROM json_each(?12) expected
        LEFT JOIN space_videos membership
          ON membership.space_id = ?6
         AND membership.video_id = json_extract(expected.value, '$.id')
        WHERE (membership.video_id IS NOT NULL) <> CAST(json_extract(expected.value, '$.present') AS INTEGER)
           OR COALESCE(membership.folder_id, '') <> COALESCE(json_extract(expected.value, '$.folder_id'), '')
           OR COALESCE(membership.revision, -1) <> CAST(json_extract(expected.value, '$.revision') AS INTEGER)
      ))
    )
    -- Manager authority is organization owner/admin, or an active organization
    -- member who is the exact manager of the selected real space.
    AND (
      ?13 IN ('owner', 'admin')
      OR (?13 = 'member' AND ?5 = 'space' AND ?9 = 'manager')
    )
  THEN 1 ELSE 0 END
)
