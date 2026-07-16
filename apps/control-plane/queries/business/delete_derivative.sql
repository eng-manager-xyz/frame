DELETE FROM business_derivative_manifests_v1
WHERE job_id=?1 AND organization_id=?2 AND ?3>=0 AND length(?4)=36
