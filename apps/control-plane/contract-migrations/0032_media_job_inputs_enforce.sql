PRAGMA foreign_keys = ON;

-- Contract phase for migration 0027. This file is not part of Wrangler's
-- normal expand-only migration directory. The protected contract gate applies
-- it only after exact-source observation proves both N and N-1 Workers use the
-- immutable-input writer.

-- Contract cannot discard work admitted by an older Worker. The protected
-- gate must first stop legacy admission, observe the exact N/N-1 bundles, let
-- active leases finish, and run the normal queued-authority reaper. This
-- immutable assertion makes the migration itself refuse if any residual
-- nonterminal native job lacks the new authority.
CREATE TABLE media_job_input_contract_assertions_v1 (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  assertion TEXT NOT NULL CHECK (assertion = 'legacy_native_work_drained'),
  asserted_at_ms INTEGER NOT NULL CHECK (
    asserted_at_ms BETWEEN 1 AND 9007199254740991
  )
) WITHOUT ROWID;

CREATE TRIGGER media_job_input_contract_assertions_v1_guard
BEFORE INSERT ON media_job_input_contract_assertions_v1
WHEN NOT EXISTS (
  SELECT 1 FROM media_job_input_rollout_v1
  WHERE singleton = 1 AND phase = 'expand' AND updated_at_ms = 0
) OR EXISTS (
  SELECT 1 FROM media_jobs j
  WHERE j.selected_executor = 'native_gstreamer'
    AND j.state IN ('queued', 'leased', 'running')
    AND NOT (
      j.input_contract_version = 1
      AND (SELECT COUNT(*) FROM media_job_inputs_v1 bound
            WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id)
          BETWEEN 1 AND 64
      AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 current
            WHERE current.job_id = j.id AND current.organization_id = j.organization_id) =
          (SELECT COUNT(*) FROM media_job_inputs_v1 bound
            WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id)
      AND (SELECT MIN(current.ordinal) FROM media_job_current_inputs_v1 current
            WHERE current.job_id = j.id AND current.organization_id = j.organization_id) = 0
      AND (SELECT MAX(current.ordinal) FROM media_job_current_inputs_v1 current
            WHERE current.job_id = j.id AND current.organization_id = j.organization_id) =
          (SELECT COUNT(*) - 1 FROM media_job_inputs_v1 bound
            WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id)
      AND CASE json_extract(j.payload_json, '$.profile')
        WHEN 'segment_mux_v1' THEN
          (SELECT COUNT(*) FROM media_job_inputs_v1 bound
            WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id)
            BETWEEN 2 AND 64
        WHEN 'composition_v1' THEN
          (SELECT COUNT(*) FROM media_job_inputs_v1 bound
            WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id)
            BETWEEN 1 AND 64
        ELSE (SELECT COUNT(*) FROM media_job_inputs_v1 bound
          WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) = 1
      END
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_contract_not_drained_v1');
END;

CREATE TRIGGER media_job_input_contract_assertions_v1_immutable
BEFORE UPDATE ON media_job_input_contract_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_contract_assertion_v1');
END;

CREATE TRIGGER media_job_input_contract_assertions_v1_no_delete
BEFORE DELETE ON media_job_input_contract_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_contract_assertion_v1');
END;

INSERT INTO media_job_input_contract_assertions_v1(
  singleton, assertion, asserted_at_ms
) VALUES (1, 'legacy_native_work_drained', 1);

UPDATE media_job_input_rollout_v1
SET phase = 'enforced', updated_at_ms = 1
WHERE singleton = 1 AND phase = 'expand' AND updated_at_ms = 0;

CREATE TRIGGER media_jobs_v1_input_rollout_insert
BEFORE INSERT ON media_jobs
WHEN NEW.selected_executor = 'native_gstreamer'
  AND NEW.state = 'queued'
  AND (
    NEW.input_contract_version IS NOT 1
    OR NOT (
      NEW.source_version IS NOT NULL
      AND NEW.video_id IS NOT NULL
      AND (
        (COALESCE(json_type(NEW.payload_json, '$.source_inputs'), '') = 'array'
          AND json_array_length(NEW.payload_json, '$.source_inputs') BETWEEN 1 AND 64
          AND COALESCE(json_extract(NEW.payload_json, '$.source_inputs[0].video_id'), '') = NEW.video_id
          AND COALESCE(json_extract(NEW.payload_json, '$.source_inputs[0].source_version'), 0) = NEW.source_version
          AND CASE json_extract(NEW.payload_json, '$.profile')
            WHEN 'segment_mux_v1' THEN json_array_length(NEW.payload_json, '$.source_inputs') BETWEEN 2 AND 64
            WHEN 'composition_v1' THEN json_array_length(NEW.payload_json, '$.source_inputs') BETWEEN 1 AND 64
            ELSE json_array_length(NEW.payload_json, '$.source_inputs') = 1
          END)
        OR (json_type(NEW.payload_json, '$.source_inputs') IS NULL
          AND json_extract(NEW.payload_json, '$.profile') != 'composition_v1')
      )
    )
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_rollout_v1');
END;

CREATE TRIGGER media_jobs_v1_input_rollout_update
BEFORE UPDATE OF state, selected_executor, payload_json ON media_jobs
WHEN NEW.selected_executor = 'native_gstreamer'
  AND NEW.state = 'queued'
  AND (
    NEW.input_contract_version IS NOT 1
    OR NOT (
      NEW.source_version IS NOT NULL
      AND NEW.video_id IS NOT NULL
      AND (
        (COALESCE(json_type(NEW.payload_json, '$.source_inputs'), '') = 'array'
          AND json_array_length(NEW.payload_json, '$.source_inputs') BETWEEN 1 AND 64
          AND COALESCE(json_extract(NEW.payload_json, '$.source_inputs[0].video_id'), '') = NEW.video_id
          AND COALESCE(json_extract(NEW.payload_json, '$.source_inputs[0].source_version'), 0) = NEW.source_version
          AND CASE json_extract(NEW.payload_json, '$.profile')
            WHEN 'segment_mux_v1' THEN json_array_length(NEW.payload_json, '$.source_inputs') BETWEEN 2 AND 64
            WHEN 'composition_v1' THEN json_array_length(NEW.payload_json, '$.source_inputs') BETWEEN 1 AND 64
            ELSE json_array_length(NEW.payload_json, '$.source_inputs') = 1
          END)
        OR (json_type(NEW.payload_json, '$.source_inputs') IS NULL
          AND json_extract(NEW.payload_json, '$.profile') != 'composition_v1')
      )
    )
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_rollout_v1');
END;
