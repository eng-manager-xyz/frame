INSERT INTO business_retention_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM business_data_handling_policies_v1 policy
         WHERE policy.data_class = ?3
           AND (
             (?4 = 'export' AND policy.exportable = 1)
             OR (?4 = 'delete' AND NOT EXISTS (
               SELECT 1 FROM business_legal_holds_v1 hold
               WHERE hold.organization_id = ?2
                 AND hold.data_class = ?3 AND hold.subject_id = ?5
                 AND hold.released_at_ms IS NULL
             ))
           )
       ) THEN 1 ELSE 0 END
