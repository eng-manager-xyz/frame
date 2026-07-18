SELECT v.owner_id,
       (
         SELECT shared.organization_id
         FROM shared_videos shared
         WHERE shared.video_id = v.id
           AND shared.revoked_at_ms IS NULL
         LIMIT 1
       ) AS shared_organization_id
FROM videos v
WHERE v.id = ?1
LIMIT 1;
