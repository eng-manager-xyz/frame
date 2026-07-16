INSERT INTO business_repository_assertions_v1(id, satisfied)
SELECT ?1, CASE WHEN EXISTS (
  SELECT 1 FROM business_derivative_manifests_v1
  WHERE job_id=?2 AND organization_id=?3 AND executor=?4 AND source_object_id=?5
    AND source_version=?6 AND transform_profile=?7 AND profile_version=?8
    AND output_role=?9 AND output_object_id IS ?10 AND output_object_key=?11
    AND output_checksum IS ?12 AND output_content_type=?13 AND state=?14
    AND usage_units=?15 AND cost_microcredits=?16 AND failure_class IS ?17
    AND revision=?18 AND last_operation_id=?19
) THEN 1 ELSE 0 END
