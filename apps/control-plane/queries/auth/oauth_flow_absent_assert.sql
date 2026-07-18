INSERT INTO auth_repository_assertions_v2(id, satisfied)
SELECT ?2,
       CASE WHEN NOT EXISTS (
         SELECT 1
         FROM auth_oauth_flows_v2 flow
         JOIN json_each(?1) candidate
           ON CAST(json_extract(candidate.value, '$.key_version') AS INTEGER) = flow.state_key_version
          AND CAST(json_extract(candidate.value, '$.digest') AS TEXT) = flow.state_digest
       ) THEN 1 ELSE 0 END
