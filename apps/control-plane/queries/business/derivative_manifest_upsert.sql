INSERT INTO business_derivative_manifests_v1(
  job_id, organization_id, executor, source_object_id, source_version,
  transform_profile, profile_version, output_role, output_object_id,
  output_object_key, output_checksum, output_content_type, state, usage_units,
  cost_microcredits, failure_class, revision, last_operation_id
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?19)
ON CONFLICT(job_id) DO UPDATE SET
  executor = excluded.executor,
  output_object_id = excluded.output_object_id,
  output_checksum = excluded.output_checksum,
  state = excluded.state,
  usage_units = excluded.usage_units,
  cost_microcredits = excluded.cost_microcredits,
  failure_class = excluded.failure_class,
  revision = excluded.revision,
  last_operation_id = excluded.last_operation_id
WHERE business_derivative_manifests_v1.organization_id = excluded.organization_id
  AND business_derivative_manifests_v1.executor = excluded.executor
  AND business_derivative_manifests_v1.source_object_id = excluded.source_object_id
  AND business_derivative_manifests_v1.source_version = excluded.source_version
  AND business_derivative_manifests_v1.transform_profile = excluded.transform_profile
  AND business_derivative_manifests_v1.profile_version = excluded.profile_version
  AND business_derivative_manifests_v1.output_role = excluded.output_role
  AND business_derivative_manifests_v1.output_object_key = excluded.output_object_key
  AND business_derivative_manifests_v1.output_content_type = excluded.output_content_type
  AND business_derivative_manifests_v1.revision = ?18
