INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN (
         ?3 = 'user'
         AND EXISTS (
           SELECT 1 FROM organizations o
           JOIN organization_members m
             ON m.organization_id = o.id AND m.user_id = ?4 AND m.state = 'active'
           JOIN users u ON u.id = m.user_id AND u.status = 'active'
           JOIN auth_identities_v2 i ON i.user_id = u.id
           WHERE o.id = ?2 AND o.status = 'active'
             AND i.identity_revision = ?5 AND i.session_version = ?6
             AND (?7 = '' OR EXISTS (
               SELECT 1 FROM videos v
               WHERE v.id = ?7 AND v.organization_id = o.id AND v.deleted_at_ms IS NULL
                 AND (
                   v.privacy <> 'private' OR v.owner_id = ?4
                   OR m.role IN ('owner','admin')
                 )
             ))
         )
       ) OR (
         ?3 = 'anonymous' AND length(?4) = 64 AND ?7 <> ''
         AND EXISTS (
           SELECT 1 FROM videos v
           WHERE v.id = ?7 AND v.organization_id = ?2
             AND v.deleted_at_ms IS NULL AND v.privacy IN ('public','unlisted')
         )
       ) THEN 1 ELSE 0 END
