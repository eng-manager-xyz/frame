INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1
         FROM users u
         JOIN organizations o ON o.id = ?3
         LEFT JOIN organization_members m
           ON m.organization_id = o.id AND m.user_id = u.id
         WHERE u.id = ?2
           AND u.status = 'active'
           AND (?4 = 'any' OR (?4 = 'active_only' AND o.status = 'active'))
           AND (
             (?5 = 'active_membership' AND m.state = 'active')
             OR (
               ?5 = 'owner_or_active_membership'
               AND (o.owner_id = u.id OR m.state = 'active')
             )
           )
       ) THEN 1 ELSE 0 END
