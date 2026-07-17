PRAGMA foreign_keys = ON;

ALTER TABLE media_jobs ADD COLUMN input_contract_version INTEGER
  CHECK (input_contract_version IS NULL OR input_contract_version = 1);

-- Runtime-readable rollout state keeps N payloads parseable by the released
-- N-1 Worker. Multi-source/composition admission stays closed until the
-- protected contract phase proves both the active and rollback bundles
-- understand the new payload. Singleton jobs carry their authority in the new
-- input table but omit the new JSON field during the expand window.
CREATE TABLE media_job_input_rollout_v1 (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  phase TEXT NOT NULL CHECK (phase IN ('expand', 'enforced')),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN 0 AND 9007199254740991
  )
) WITHOUT ROWID;

INSERT INTO media_job_input_rollout_v1(singleton, phase, updated_at_ms)
VALUES (1, 'expand', 0);

CREATE TRIGGER media_job_input_rollout_v1_transition
BEFORE UPDATE ON media_job_input_rollout_v1
WHEN NEW.singleton != OLD.singleton
  OR NOT (
    OLD.phase = 'expand' AND NEW.phase = 'enforced'
    AND NEW.updated_at_ms > OLD.updated_at_ms
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_rollout_phase_v1');
END;

CREATE TRIGGER media_job_input_rollout_v1_no_delete
BEFORE DELETE ON media_job_input_rollout_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_rollout_phase_v1');
END;

-- Immutable, dense source authority for native media profiles. Existing jobs
-- deliberately receive no inferred inputs: only a newly validated request may
-- bind object identities, and claim selection requires the complete set. The
-- rollout disposition below moves pre-migration nonterminal native jobs to an
-- explicit terminal failure instead of leaving them permanently unclaimable.
CREATE TABLE media_job_inputs_v1 (
  job_id TEXT NOT NULL REFERENCES media_jobs(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 63),
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  source_version INTEGER NOT NULL CHECK (source_version BETWEEN 1 AND 9007199254740991),
  object_key TEXT NOT NULL REFERENCES object_manifests(object_key) ON DELETE RESTRICT,
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64
      AND lower(checksum_sha256) = checksum_sha256
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  content_type TEXT NOT NULL CHECK (content_type IN (
    'video/mp4', 'video/quicktime', 'video/webm', 'video/x-matroska',
    'audio/mpeg', 'audio/mp4', 'audio/wav', 'audio/webm', 'audio/ogg'
  )),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (job_id, ordinal),
  -- Composition v1 may intentionally bind the same immutable source at more
  -- than one ordinal. Other profiles reject duplicates in the authority
  -- trigger below. Ordinal is therefore the durable occurrence identity.
  CHECK (
    substr(object_key, 1, length('tenants/' || organization_id || '/videos/' || video_id || '/')) =
      'tenants/' || organization_id || '/videos/' || video_id || '/'
  ),
  CHECK (
    instr(object_key, '..') = 0 AND instr(object_key, char(92)) = 0
      AND instr(object_key, '?') = 0 AND instr(object_key, '#') = 0
      AND instr(object_key, '%') = 0
  )
);

CREATE INDEX media_job_inputs_v1_job_order_idx
  ON media_job_inputs_v1(organization_id, job_id, ordinal);

CREATE TRIGGER media_job_inputs_v1_authority
BEFORE INSERT ON media_job_inputs_v1
WHEN NOT EXISTS (
  SELECT 1
  FROM media_jobs j
  JOIN videos v
    ON v.id = NEW.video_id AND v.organization_id = NEW.organization_id
   AND v.deleted_at_ms IS NULL
  JOIN object_manifests m
    ON m.object_key = NEW.object_key
   AND m.organization_id = NEW.organization_id
   AND m.video_id = NEW.video_id
   AND m.object_version = NEW.source_version
   AND m.bytes = NEW.bytes
   AND m.checksum_sha256 = NEW.checksum_sha256
   AND m.content_type = NEW.content_type
   AND m.role IN ('source', 'import')
   AND m.state = 'available'
  JOIN storage_governed_objects_v1 g
    ON g.organization_id = NEW.organization_id
   AND g.object_key = NEW.object_key
   AND g.role = 'source'
   AND g.state = 'active'
   AND g.malware_disposition = 'clean'
   AND g.immutable_revision = NEW.source_version
   AND g.checksum_sha256 = NEW.checksum_sha256
   AND g.bytes = NEW.bytes
   AND g.content_type = NEW.content_type
  WHERE j.id = NEW.job_id
    AND j.organization_id = NEW.organization_id
    AND j.state = 'queued'
    AND j.attempt = 0
    AND j.input_contract_version = 1
    AND (
      (json_type(j.payload_json, '$.source_inputs') = 'array'
        AND json_array_length(j.payload_json, '$.source_inputs') BETWEEN 1 AND 64
        AND NEW.ordinal < json_array_length(j.payload_json, '$.source_inputs')
        AND json_extract(j.payload_json, '$.source_inputs[' || NEW.ordinal || '].video_id') = NEW.video_id
        AND json_extract(j.payload_json, '$.source_inputs[' || NEW.ordinal || '].source_version') = NEW.source_version)
      OR (json_type(j.payload_json, '$.source_inputs') IS NULL
        AND NEW.ordinal = 0
        AND NEW.video_id = j.video_id
        AND NEW.source_version = j.source_version
        AND json_extract(j.payload_json, '$.profile') != 'composition_v1')
    )
    AND (
      (NEW.ordinal = 0 AND j.video_id = NEW.video_id AND j.source_version = NEW.source_version)
      OR NEW.ordinal > 0
    )
    AND CASE json_extract(j.payload_json, '$.profile')
      WHEN 'segment_mux_v1' THEN json_array_length(j.payload_json, '$.source_inputs') BETWEEN 2 AND 64
      WHEN 'composition_v1' THEN json_array_length(j.payload_json, '$.source_inputs') BETWEEN 1 AND 64
      ELSE json_type(j.payload_json, '$.source_inputs') IS NULL
        OR json_array_length(j.payload_json, '$.source_inputs') = 1
    END
    AND (
      json_extract(j.payload_json, '$.profile') = 'composition_v1'
      OR NOT EXISTS (
        SELECT 1 FROM media_job_inputs_v1 prior
        WHERE prior.job_id = NEW.job_id
          AND prior.video_id = NEW.video_id
          AND prior.source_version = NEW.source_version
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_authority_v1');
END;

CREATE TRIGGER media_job_inputs_v1_immutable
BEFORE UPDATE ON media_job_inputs_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_immutable_v1');
END;

CREATE TRIGGER media_job_inputs_v1_no_delete
BEFORE DELETE ON media_job_inputs_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_media_job_input_immutable_v1');
END;

-- This view is the sole worker-facing input authority. It revalidates every
-- immutable binding against current video, manifest, governance, and payload
-- state, so a tombstone, quarantine, manifest rewrite, or ordinal drift
-- disappears before claim and source delivery.
CREATE VIEW media_job_current_inputs_v1 AS
SELECT i.job_id,
       i.organization_id,
       i.ordinal,
       i.video_id,
       i.source_version,
       i.object_key,
       i.bytes,
       i.checksum_sha256,
       i.content_type,
       i.created_at_ms
FROM media_job_inputs_v1 i
JOIN media_jobs j
  ON j.id = i.job_id AND j.organization_id = i.organization_id
JOIN videos v
  ON v.id = i.video_id AND v.organization_id = i.organization_id
 AND v.deleted_at_ms IS NULL
JOIN object_manifests m
  ON m.object_key = i.object_key
 AND m.organization_id = i.organization_id
 AND m.video_id = i.video_id
 AND m.object_version = i.source_version
 AND m.bytes = i.bytes
 AND m.checksum_sha256 = i.checksum_sha256
 AND m.content_type = i.content_type
 AND m.role IN ('source', 'import')
 AND m.state = 'available'
JOIN storage_governed_objects_v1 g
  ON g.organization_id = i.organization_id
 AND g.object_key = i.object_key
 AND g.role = 'source'
 AND g.state = 'active'
 AND g.malware_disposition = 'clean'
 AND g.immutable_revision = i.source_version
 AND g.checksum_sha256 = i.checksum_sha256
 AND g.bytes = i.bytes
 AND g.content_type = i.content_type
WHERE j.input_contract_version = 1
  AND (
    (json_type(j.payload_json, '$.source_inputs') = 'array'
      AND json_array_length(j.payload_json, '$.source_inputs') BETWEEN 1 AND 64
      AND i.ordinal < json_array_length(j.payload_json, '$.source_inputs')
      AND json_extract(j.payload_json, '$.source_inputs[' || i.ordinal || '].video_id') = i.video_id
      AND json_extract(j.payload_json, '$.source_inputs[' || i.ordinal || '].source_version') = i.source_version)
    OR (json_type(j.payload_json, '$.source_inputs') IS NULL
      AND i.ordinal = 0
      AND i.video_id = j.video_id
      AND i.source_version = j.source_version
      AND json_extract(j.payload_json, '$.profile') != 'composition_v1')
  )
  AND (
    (i.ordinal = 0 AND j.video_id = i.video_id AND j.source_version = i.source_version)
    OR i.ordinal > 0
  );

-- Legacy disposition and mixed-version enforcement are deliberately delayed.
-- Expansion cannot terminate an in-flight N-1 worker after its provider side
-- effect. The claim-aware runtime ignores missing input authority, reaps queued
-- rows under its normal fenced path, and lets already leased work finish. The
-- protected contract migration reconciles any residual only after N/N-1
-- compatibility observation. The protected contract
-- migration 0032 may run only after both the active Worker and its approved
-- rollback use the immutable-input writer. Until then, the new runtime's
-- queued-authority reaper terminalizes any legacy row admitted during the
-- expand/deploy window without breaking the previous Worker before deploy.
