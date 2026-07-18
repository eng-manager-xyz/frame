INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN (
         ?3 = 'user'
         AND EXISTS (
           SELECT 1
           FROM users u
           JOIN auth_identities_v2 i ON i.user_id = u.id
           JOIN organizations o ON o.id = ?2
           JOIN organization_members m
             ON m.organization_id = o.id AND m.user_id = u.id
           WHERE u.id = ?4 AND u.status = 'active'
             AND i.identity_revision = ?5 AND i.session_version = ?6
             AND o.status = 'active'
             AND o.revision = ?7 AND o.authority_version = ?8
             AND m.state = 'active'
             AND m.revision = ?9 AND m.authority_version = ?10
             AND (
               (?11 = 'member' AND m.role IN ('owner','admin','member','viewer'))
               OR (?11 = 'write' AND m.role IN ('owner','admin','member'))
               OR (?11 = 'admin' AND m.role IN ('owner','admin'))
               OR (?11 = 'owner' AND m.role = 'owner')
             )
             AND (
               ?12 NOT IN ('video_manage','edit_manage','share_manage','comment_create','comment_delete')
               OR (
                 ?12 IN ('video_manage','edit_manage','share_manage','comment_delete')
                 AND (
                   m.role IN ('owner','admin')
                   OR EXISTS (
                     SELECT 1 FROM videos owned
                     WHERE owned.id = ?13 AND owned.organization_id = o.id
                       AND owned.owner_id = u.id AND owned.deleted_at_ms IS NULL
                   )
                  OR (
                    ?12 = 'comment_delete' AND EXISTS (
                      SELECT 1 FROM comments target
                      JOIN videos owned ON owned.id = target.video_id
                      WHERE target.id = ?13 AND target.organization_id = o.id
                        AND target.deleted_at_ms IS NULL
                        AND (target.author_user_id = u.id OR owned.owner_id = u.id)
                    )
                  )
                 )
               )
               OR (
                 ?12 = 'comment_create'
                 AND EXISTS (
                   SELECT 1 FROM videos commentable
                   WHERE commentable.id = ?13 AND commentable.organization_id = o.id
                     AND commentable.deleted_at_ms IS NULL
                     AND commentable.comments_enabled = 1
                     AND (
                       commentable.privacy <> 'private'
                       OR commentable.owner_id = u.id
                       OR m.role IN ('owner','admin')
                     )
                 )
               )
             )
         )
       ) OR (
         ?3 = 'anonymous' AND length(?4) = 64
         AND ?12 = 'comment_create'
         AND EXISTS (
           SELECT 1 FROM videos v
           WHERE v.id = ?13 AND v.organization_id = ?2
             AND v.deleted_at_ms IS NULL AND v.comments_enabled = 1
             AND v.privacy IN ('public','unlisted')
         )
       ) THEN 1 ELSE 0 END
