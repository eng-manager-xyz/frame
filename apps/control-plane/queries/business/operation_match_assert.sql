INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1,
       CASE WHEN EXISTS (
         SELECT 1 FROM business_repository_operations_v1
         WHERE organization_id=?2 AND principal_kind=?3 AND principal_subject=?4
           AND idempotency_key=?5 AND action=?6 AND subject_id=?7
           AND request_fingerprint=?8 AND result_code='accepted'
       ) THEN 1 ELSE 0 END
