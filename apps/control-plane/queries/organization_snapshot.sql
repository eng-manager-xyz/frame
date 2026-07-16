SELECT o.id,
       o.name,
       o.status,
       o.revision,
       (
         SELECT COUNT(*)
         FROM organization_members m
         WHERE m.organization_id = o.id
           AND m.state = 'active'
       ) AS active_members,
       (
         SELECT COUNT(*)
         FROM videos v
         WHERE v.organization_id = o.id
           AND v.deleted_at_ms IS NULL
       ) AS active_videos,
       (
         SELECT COUNT(*)
         FROM video_uploads u
         JOIN videos v
           ON v.id = u.video_id
          AND v.organization_id = u.organization_id
         WHERE u.organization_id = o.id
           AND u.state NOT IN ('complete', 'failed', 'aborted')
       ) AS active_uploads,
       (
         SELECT COUNT(*)
         FROM media_jobs j
         JOIN videos v
           ON v.id = j.video_id
          AND v.organization_id = j.organization_id
         WHERE j.organization_id = o.id
           AND j.state IN ('queued', 'leased', 'running')
       ) AS active_media_jobs
FROM organizations o
WHERE o.id = ?1
  AND o.status <> 'deleted'
LIMIT 1
