INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?4,
       CASE WHEN EXISTS (
         SELECT 1
         FROM auth_oauth_flows_v2 flow
         WHERE flow.id = ?1
           AND flow.revision = ?2
           AND flow.consumed_at_ms IS ?3
       ) THEN 1 ELSE 0 END
