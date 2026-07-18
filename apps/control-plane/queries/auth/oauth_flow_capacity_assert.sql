INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?3,
       CASE WHEN (
         SELECT COUNT(*)
         FROM auth_oauth_flows_v2 flow
         WHERE flow.expires_at_ms > ?1
       ) >= 4096 AND (
         SELECT MIN(flow.expires_at_ms)
         FROM auth_oauth_flows_v2 flow
         WHERE flow.expires_at_ms > ?1
       ) = ?2 THEN 1 ELSE 0 END
