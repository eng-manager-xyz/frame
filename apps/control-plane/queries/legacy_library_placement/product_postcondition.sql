UPDATE authenticated_web_action_assertions_v1
SET actual_count = CASE WHEN NOT EXISTS (
  SELECT 1
  FROM json_each(?5) expected
  LEFT JOIN videos video
    ON video.id = json_extract(expected.value, '$.id')
   AND video.organization_id = ?2
   AND video.deleted_at_ms IS NULL
  WHERE (video.id IS NOT NULL) <> CAST(json_extract(expected.value, '$.video_present') AS INTEGER)
     OR COALESCE(video.folder_id, '') <> COALESCE(json_extract(expected.value, '$.video_folder_id'), '')
     OR COALESCE(video.revision, -1) <> CAST(json_extract(expected.value, '$.video_revision') AS INTEGER)
     OR (
       ?3 = 'organization' AND (
         (
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
       )
     )
     OR (
       ?3 = 'space' AND (
         (
           SELECT COUNT(*) FROM space_videos membership
           WHERE membership.space_id = ?4
             AND membership.video_id = json_extract(expected.value, '$.id')
         ) <> CAST(json_extract(expected.value, '$.scope_present') AS INTEGER)
         OR COALESCE((
           SELECT membership.folder_id FROM space_videos membership
           WHERE membership.space_id = ?4
             AND membership.video_id = json_extract(expected.value, '$.id')
         ), '') <> COALESCE(json_extract(expected.value, '$.scope_folder_id'), '')
         OR COALESCE((
           SELECT membership.revision FROM space_videos membership
           WHERE membership.space_id = ?4
             AND membership.video_id = json_extract(expected.value, '$.id')
         ), -1) <> CAST(json_extract(expected.value, '$.scope_revision') AS INTEGER)
       )
     )
)
THEN 1 ELSE 0 END
WHERE operation_id = ?1
  AND assertion_kind = 'product_effect'
