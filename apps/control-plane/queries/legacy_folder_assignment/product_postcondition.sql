UPDATE authenticated_web_action_assertions_v1
SET actual_count = CASE WHEN NOT EXISTS (
  SELECT 1
  FROM json_each(?3) expected
  LEFT JOIN videos v
    ON v.id = json_extract(expected.value, '$.id')
   AND v.organization_id = ?2
   AND v.deleted_at_ms IS NULL
  WHERE v.id IS NULL
     OR v.folder_id IS NOT json_extract(expected.value, '$.video_folder_id')
     OR v.revision <> CAST(json_extract(expected.value, '$.video_revision') AS INTEGER)
     OR (
       ?4 = 'space' AND (
         (
           SELECT COUNT(*) FROM space_videos sv
           WHERE sv.space_id = json_extract(expected.value, '$.scope_id')
             AND sv.video_id = json_extract(expected.value, '$.id')
         ) <> CAST(json_extract(expected.value, '$.scope_present') AS INTEGER)
         OR COALESCE((
           SELECT sv.folder_id FROM space_videos sv
           WHERE sv.space_id = json_extract(expected.value, '$.scope_id')
             AND sv.video_id = json_extract(expected.value, '$.id')
         ), '') <> COALESCE(json_extract(expected.value, '$.scope_folder_id'), '')
         OR COALESCE((
           SELECT sv.revision FROM space_videos sv
           WHERE sv.space_id = json_extract(expected.value, '$.scope_id')
             AND sv.video_id = json_extract(expected.value, '$.id')
         ), -1) <> CAST(json_extract(expected.value, '$.scope_revision') AS INTEGER)
       )
     )
     OR (
       ?4 = 'organization' AND (
         (
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
       )
     )
)
THEN 1 ELSE 0 END
WHERE operation_id = ?1
  AND assertion_kind = 'product_effect'
