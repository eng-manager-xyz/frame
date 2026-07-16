SELECT v.id,
       v.owner_id,
       v.state,
       v.privacy,
       v.revision,
       m.role AS actor_role,
       EXISTS (
         SELECT 1
         FROM space_videos sv
         JOIN spaces s
           ON s.id = sv.space_id
          AND s.organization_id = v.organization_id
          AND s.deleted_at_ms IS NULL
         JOIN space_members sm
           ON sm.space_id = s.id
         WHERE sv.video_id = v.id
           AND sm.user_id = ?3
           AND sm.role = 'manager'
       ) AS actor_manages_space
FROM videos v
JOIN organizations o
  ON o.id = v.organization_id
 AND o.status = 'active'
JOIN organization_members m
  ON m.organization_id = v.organization_id
 AND m.user_id = ?3
 AND m.state = 'active'
WHERE v.id = ?1
  AND v.organization_id = ?2
  AND v.deleted_at_ms IS NULL
LIMIT 1
