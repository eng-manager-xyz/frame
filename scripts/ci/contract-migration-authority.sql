-- Privacy-safe production authority snapshot. This query returns only schema,
-- rollout, and aggregate drain state; it never selects tenant identifiers or
-- provider handles. The protected contract workflow verifies the exact row
-- before and after applying contract migrations.
SELECT
  COALESCE((
    SELECT json_group_array(name)
    FROM (SELECT name FROM d1_migrations ORDER BY id)
  ), '[]') AS migration_names_json,
  COALESCE((
    SELECT phase FROM media_job_input_rollout_v1 WHERE singleton = 1
  ), 'missing') AS media_rollout_phase,
  COALESCE((
    SELECT phase FROM r2_multipart_claim_rollout_v1 WHERE singleton = 1
  ), 'missing') AS r2_rollout_phase,
  (
    SELECT COUNT(*)
    FROM media_jobs job
    WHERE job.selected_executor = 'native_gstreamer'
      AND job.state IN ('queued', 'leased', 'running')
      AND NOT (
        job.input_contract_version = 1
        AND (SELECT COUNT(*) FROM media_job_inputs_v1 bound
              WHERE bound.job_id = job.id
                AND bound.organization_id = job.organization_id) BETWEEN 1 AND 64
        AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 current
              WHERE current.job_id = job.id
                AND current.organization_id = job.organization_id) =
            (SELECT COUNT(*) FROM media_job_inputs_v1 bound
              WHERE bound.job_id = job.id
                AND bound.organization_id = job.organization_id)
        AND (SELECT MIN(current.ordinal) FROM media_job_current_inputs_v1 current
              WHERE current.job_id = job.id
                AND current.organization_id = job.organization_id) = 0
        AND (SELECT MAX(current.ordinal) FROM media_job_current_inputs_v1 current
              WHERE current.job_id = job.id
                AND current.organization_id = job.organization_id) =
            (SELECT COUNT(*) - 1 FROM media_job_inputs_v1 bound
              WHERE bound.job_id = job.id
                AND bound.organization_id = job.organization_id)
        AND CASE json_extract(job.payload_json, '$.profile')
          WHEN 'segment_mux_v1' THEN
            (SELECT COUNT(*) FROM media_job_inputs_v1 bound
              WHERE bound.job_id = job.id
                AND bound.organization_id = job.organization_id) BETWEEN 2 AND 64
          WHEN 'composition_v1' THEN
            (SELECT COUNT(*) FROM media_job_inputs_v1 bound
              WHERE bound.job_id = job.id
                AND bound.organization_id = job.organization_id) BETWEEN 1 AND 64
          ELSE (SELECT COUNT(*) FROM media_job_inputs_v1 bound
            WHERE bound.job_id = job.id
              AND bound.organization_id = job.organization_id) = 1
        END
      )
  ) AS media_legacy_residual_count,
  (
    SELECT COUNT(*) FROM r2_multipart_sessions_v1
    WHERE state IN ('open', 'completing')
  ) AS r2_nonterminal_session_count,
  (
    SELECT COUNT(*) FROM r2_multipart_creation_claims_v1
    WHERE state IN ('reserved', 'provider_bound')
  ) AS r2_uncommitted_creation_count,
  (
    SELECT COUNT(*)
    FROM r2_multipart_part_claims_v1 claim
    LEFT JOIN r2_multipart_parts_v1 part
      ON part.upload_id = claim.upload_id
     AND part.part_number = claim.part_number
     AND part.part_claim_token = claim.claim_token
    WHERE part.upload_id IS NULL
  ) AS r2_unmaterialized_part_count,
  (
    SELECT COUNT(*) FROM r2_multipart_completion_claims_v1
    WHERE state = 'active'
  ) AS r2_active_completion_count,
  (
    SELECT COUNT(*) FROM r2_multipart_completion_reconciliation_v1
    WHERE state = 'pending'
  ) AS r2_pending_completion_reconciliation_count,
  (
    SELECT COUNT(*) FROM r2_multipart_abort_reconciliation_v1
    WHERE state = 'pending'
  ) AS r2_pending_abort_reconciliation_count,
  (
    SELECT COUNT(*) FROM sqlite_master
    WHERE type = 'table'
      AND name IN (
        'media_job_input_contract_assertions_v1',
        'r2_multipart_claim_contract_assertions_v1'
      )
  ) AS contract_assertion_table_count,
  (
    SELECT COUNT(*) FROM sqlite_master
    WHERE type = 'trigger'
      AND name IN (
        'media_job_input_contract_assertions_v1_guard',
        'media_job_input_contract_assertions_v1_immutable',
        'media_job_input_contract_assertions_v1_no_delete',
        'media_jobs_v1_input_rollout_insert',
        'media_jobs_v1_input_rollout_update',
        'r2_multipart_claim_contract_assertions_v1_guard',
        'r2_multipart_claim_contract_assertions_v1_immutable',
        'r2_multipart_claim_contract_assertions_v1_no_delete',
        'r2_multipart_sessions_v1_creation_authority',
        'r2_multipart_abort_reconciliation_v1_completion_exclusion',
        'r2_multipart_parts_v1_claim_authority',
        'r2_multipart_parts_v1_claim_immutable',
        'r2_multipart_completions_v1_session_authority',
        'r2_multipart_sessions_v1_state_transition'
      )
  ) AS contract_enforcement_trigger_count;
