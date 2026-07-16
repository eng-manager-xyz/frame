PRAGMA foreign_keys = ON;

-- The catalog is application-versioned, while this table is the durable
-- operator control plane. A profile can be killed or redirected without
-- changing the public media-job contract or mutating an in-flight artifact.
CREATE TABLE media_profile_policies_v1 (
  profile_id TEXT PRIMARY KEY NOT NULL,
  catalog_version INTEGER NOT NULL CHECK (catalog_version = 1),
  route_mode TEXT NOT NULL CHECK (route_mode IN (
    'managed_preferred', 'native_only', 'external_provider', 'control_plane', 'disabled'
  )),
  fallback_executor TEXT CHECK (
    fallback_executor IS NULL OR fallback_executor IN ('native_gstreamer')
  ),
  managed_operation_budget INTEGER NOT NULL
    CHECK (managed_operation_budget BETWEEN 0 AND 9007199254740991),
  native_concurrency_budget INTEGER NOT NULL
    CHECK (native_concurrency_budget BETWEEN 0 AND 100000),
  data_residency_policy TEXT NOT NULL CHECK (
    data_residency_policy IN ('private_r2_binding', 'approved_native_region', 'declared_provider')
  ),
  revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK ((route_mode = 'managed_preferred') = (fallback_executor = 'native_gstreamer')),
  CHECK (profile_id = lower(profile_id) AND profile_id GLOB '*_v[0-9]*'
         AND length(profile_id) BETWEEN 4 AND 64)
);

INSERT INTO media_profile_policies_v1(
  profile_id, catalog_version, route_mode, fallback_executor,
  managed_operation_budget, native_concurrency_budget,
  data_residency_policy, revision, updated_at_ms
) VALUES
  ('optimized_clip_v1', 1, 'managed_preferred', 'native_gstreamer', 1000000, 64, 'private_r2_binding', 1, 0),
  ('thumbnail_v1', 1, 'managed_preferred', 'native_gstreamer', 1000000, 64, 'private_r2_binding', 1, 0),
  ('spritesheet_v1', 1, 'managed_preferred', 'native_gstreamer', 1000000, 64, 'private_r2_binding', 1, 0),
  ('audio_extract_v1', 1, 'managed_preferred', 'native_gstreamer', 1000000, 64, 'private_r2_binding', 1, 0),
  ('probe_v1', 1, 'native_only', NULL, 0, 64, 'approved_native_region', 1, 0),
  ('audio_presence_v1', 1, 'native_only', NULL, 0, 64, 'approved_native_region', 1, 0),
  ('distribution_master_v1', 1, 'native_only', NULL, 0, 16, 'approved_native_region', 1, 0),
  ('animated_preview_v1', 1, 'native_only', NULL, 0, 32, 'approved_native_region', 1, 0),
  ('audio_normalize_v1', 1, 'native_only', NULL, 0, 32, 'approved_native_region', 1, 0),
  ('remux_repair_v1', 1, 'native_only', NULL, 0, 16, 'approved_native_region', 1, 0),
  ('segment_mux_v1', 1, 'native_only', NULL, 0, 16, 'approved_native_region', 1, 0),
  ('waveform_v1', 1, 'native_only', NULL, 0, 32, 'approved_native_region', 1, 0),
  ('composition_v1', 1, 'native_only', NULL, 0, 8, 'approved_native_region', 1, 0),
  ('normalize_v1', 1, 'native_only', NULL, 0, 16, 'approved_native_region', 1, 0),
  ('transcription_v1', 1, 'external_provider', NULL, 0, 16, 'declared_provider', 1, 0),
  ('ai_cleanup_v1', 1, 'external_provider', NULL, 0, 16, 'declared_provider', 1, 0);

-- Only a trusted server-side probe may assert codec/container/decoded bounds.
-- Upload clients never choose these fields. The source object's checksum and
-- byte length remain part of the identity so a replacement cannot reuse a
-- stale capability decision.
CREATE TABLE media_source_probes_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  source_version INTEGER NOT NULL CHECK (source_version BETWEEN 1 AND 9007199254740991),
  source_object_key TEXT NOT NULL,
  source_checksum_sha256 TEXT NOT NULL CHECK (
    length(source_checksum_sha256) = 64
      AND lower(source_checksum_sha256) = source_checksum_sha256
      AND source_checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  source_bytes INTEGER NOT NULL CHECK (source_bytes BETWEEN 1 AND 9007199254740991),
  source_content_type TEXT NOT NULL CHECK (length(source_content_type) BETWEEN 3 AND 127),
  container TEXT NOT NULL CHECK (container IN (
    'mp4', 'webm', 'matroska', 'quicktime', 'mpeg_ts', 'ogg', 'wav', 'm4a', 'mp3', 'unknown'
  )),
  video_codec TEXT NOT NULL CHECK (video_codec IN (
    'h264', 'h265', 'vp8', 'vp9', 'av1', 'prores', 'theora', 'none', 'unknown'
  )),
  audio_codec TEXT NOT NULL CHECK (audio_codec IN (
    'aac', 'mp3', 'opus', 'vorbis', 'flac', 'pcm', 'none', 'unknown'
  )),
  duration_ms INTEGER NOT NULL CHECK (duration_ms BETWEEN 1 AND 9007199254740991),
  width INTEGER NOT NULL CHECK (width BETWEEN 1 AND 65535),
  height INTEGER NOT NULL CHECK (height BETWEEN 1 AND 65535),
  frame_rate_numerator INTEGER NOT NULL
    CHECK (frame_rate_numerator BETWEEN 1 AND 1000000),
  frame_rate_denominator INTEGER NOT NULL
    CHECK (frame_rate_denominator BETWEEN 1 AND 1000000),
  decoded_bytes_upper_bound INTEGER NOT NULL
    CHECK (decoded_bytes_upper_bound BETWEEN 1 AND 9007199254740991),
  frame_count_upper_bound INTEGER NOT NULL
    CHECK (frame_count_upper_bound BETWEEN 1 AND 9007199254740991),
  track_count INTEGER NOT NULL CHECK (track_count BETWEEN 1 AND 1024),
  probe_contract_version INTEGER NOT NULL CHECK (probe_contract_version = 1),
  probe_digest TEXT NOT NULL CHECK (
    length(probe_digest) = 64 AND lower(probe_digest) = probe_digest
      AND probe_digest NOT GLOB '*[^0-9a-f]*'
  ),
  trust TEXT NOT NULL CHECK (trust IN ('verified_native_probe')),
  state TEXT NOT NULL CHECK (state IN ('verified', 'quarantined', 'superseded')),
  verified_at_ms INTEGER NOT NULL CHECK (verified_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, video_id, source_version),
  UNIQUE (organization_id, source_object_key, source_checksum_sha256),
  CHECK (substr(source_object_key, 1, length('tenants/' || organization_id || '/videos/' || video_id || '/')) =
         'tenants/' || organization_id || '/videos/' || video_id || '/'),
  CHECK (instr(source_object_key, '..') = 0 AND instr(source_object_key, char(92)) = 0
         AND instr(source_object_key, '?') = 0 AND instr(source_object_key, '#') = 0
         AND instr(source_object_key, '%') = 0),
  CHECK (updated_at_ms >= verified_at_ms)
);
CREATE INDEX media_source_probes_v1_state_idx
  ON media_source_probes_v1(organization_id, state, updated_at_ms);

CREATE TRIGGER media_source_probes_v1_tenant_video
BEFORE INSERT ON media_source_probes_v1
WHEN NOT EXISTS (
  SELECT 1 FROM videos v
   WHERE v.id = NEW.video_id AND v.organization_id = NEW.organization_id
     AND v.deleted_at_ms IS NULL
) OR NOT EXISTS (
  SELECT 1 FROM object_manifests m
   WHERE m.organization_id = NEW.organization_id
     AND m.video_id = NEW.video_id
     AND m.object_version = NEW.source_version
     AND m.object_key = NEW.source_object_key
     AND m.checksum_sha256 = NEW.source_checksum_sha256
     AND m.bytes = NEW.source_bytes
     AND m.content_type = NEW.source_content_type
     AND m.role IN ('source', 'import')
     AND m.state = 'available'
)
BEGIN
  SELECT RAISE(ABORT, 'media_source_probe_authority_mismatch');
END;

-- Replays may assert the same canonical probe, but a second worker can never
-- replace trusted facts for the same immutable source version.
CREATE TRIGGER media_source_probes_v1_conflict
BEFORE INSERT ON media_source_probes_v1
WHEN EXISTS (
  SELECT 1 FROM media_source_probes_v1 p
   WHERE p.organization_id = NEW.organization_id
     AND p.video_id = NEW.video_id
     AND p.source_version = NEW.source_version
     AND (
       p.source_object_key <> NEW.source_object_key
       OR p.source_checksum_sha256 <> NEW.source_checksum_sha256
       OR p.source_bytes <> NEW.source_bytes
       OR p.source_content_type <> NEW.source_content_type
       OR p.container <> NEW.container
       OR p.video_codec <> NEW.video_codec
       OR p.audio_codec <> NEW.audio_codec
       OR p.duration_ms <> NEW.duration_ms
       OR p.width <> NEW.width
       OR p.height <> NEW.height
       OR p.frame_rate_numerator <> NEW.frame_rate_numerator
       OR p.frame_rate_denominator <> NEW.frame_rate_denominator
       OR p.decoded_bytes_upper_bound <> NEW.decoded_bytes_upper_bound
       OR p.frame_count_upper_bound <> NEW.frame_count_upper_bound
       OR p.track_count <> NEW.track_count
       OR p.probe_contract_version <> NEW.probe_contract_version
       OR p.probe_digest <> NEW.probe_digest
       OR p.trust <> NEW.trust
       OR p.state <> NEW.state
     )
)
BEGIN
  SELECT RAISE(ABORT, 'media_source_probe_immutable_conflict');
END;

-- This row is the recovery authority for Worker-managed transformations. It
-- fences every attempt, records a staged artifact before publication, and
-- supports switching exactly once to the declared native fallback.
CREATE TABLE media_job_execution_v1 (
  job_id TEXT PRIMARY KEY NOT NULL REFERENCES media_jobs(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  source_version INTEGER NOT NULL CHECK (source_version BETWEEN 1 AND 9007199254740991),
  catalog_version INTEGER NOT NULL CHECK (catalog_version = 1),
  profile_id TEXT NOT NULL REFERENCES media_profile_policies_v1(profile_id) ON DELETE RESTRICT,
  profile_version INTEGER NOT NULL CHECK (profile_version BETWEEN 1 AND 65535),
  normalized_profile_sha256 TEXT NOT NULL CHECK (
    length(normalized_profile_sha256) = 64
      AND lower(normalized_profile_sha256) = normalized_profile_sha256
      AND normalized_profile_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  route_reason TEXT NOT NULL CHECK (route_reason IN (
    'managed_preferred', 'managed_kill_switch', 'managed_input_format',
    'managed_input_limit', 'managed_profile_limit', 'native_only',
    'external_provider', 'managed_failure'
  )),
  selected_executor TEXT NOT NULL CHECK (selected_executor IN (
    'cloudflare_media', 'native_gstreamer', 'external_provider', 'control_plane'
  )),
  fallback_executor TEXT CHECK (
    fallback_executor IS NULL OR fallback_executor = 'native_gstreamer'
  ),
  state TEXT NOT NULL CHECK (state IN (
    'queued', 'leased', 'transforming', 'staged', 'publishing',
    'cancelling', 'fallback_queued', 'succeeded', 'failed', 'cancelled', 'dead_letter'
  )),
  attempt INTEGER NOT NULL CHECK (attempt BETWEEN 0 AND 65535),
  lease_epoch INTEGER NOT NULL CHECK (lease_epoch BETWEEN 0 AND 9007199254740991),
  lease_token_digest TEXT CHECK (
    lease_token_digest IS NULL OR (
      length(lease_token_digest) = 64 AND lower(lease_token_digest) = lease_token_digest
        AND lease_token_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  lease_expires_at_ms INTEGER CHECK (
    lease_expires_at_ms IS NULL OR lease_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  staging_object_key TEXT,
  final_object_key TEXT NOT NULL,
  output_content_type TEXT NOT NULL CHECK (length(output_content_type) BETWEEN 3 AND 127),
  max_output_bytes INTEGER NOT NULL CHECK (max_output_bytes BETWEEN 1 AND 9007199254740991),
  staged_checksum_sha256 TEXT CHECK (
    staged_checksum_sha256 IS NULL OR (
      length(staged_checksum_sha256) = 64 AND lower(staged_checksum_sha256) = staged_checksum_sha256
        AND staged_checksum_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  staged_bytes INTEGER CHECK (staged_bytes IS NULL OR staged_bytes BETWEEN 1 AND 9007199254740991),
  provider_operations INTEGER NOT NULL DEFAULT 0
    CHECK (provider_operations BETWEEN 0 AND 9007199254740991),
  provider_output_seconds INTEGER NOT NULL DEFAULT 0
    CHECK (provider_output_seconds BETWEEN 0 AND 9007199254740991),
  native_cpu_millis INTEGER NOT NULL DEFAULT 0
    CHECK (native_cpu_millis BETWEEN 0 AND 9007199254740991),
  native_gpu_millis INTEGER NOT NULL DEFAULT 0
    CHECK (native_gpu_millis BETWEEN 0 AND 9007199254740991),
  scratch_byte_seconds INTEGER NOT NULL DEFAULT 0
    CHECK (scratch_byte_seconds BETWEEN 0 AND 9007199254740991),
  total_cost_microunits INTEGER NOT NULL DEFAULT 0
    CHECK (total_cost_microunits BETWEEN 0 AND 9007199254740991),
  failure_class TEXT CHECK (failure_class IS NULL OR failure_class IN (
    'invalid_input', 'security_violation', 'unsupported_format', 'quota', 'timeout',
    'provider_outage', 'output_incompatible', 'beta_regression', 'storage_failure',
    'resource_limit', 'cancelled', 'lease_lost'
  )),
  manifest_digest TEXT CHECK (
    manifest_digest IS NULL OR (
      length(manifest_digest) = 64 AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (organization_id, final_object_key),
  FOREIGN KEY (organization_id, video_id, source_version)
    REFERENCES media_source_probes_v1(organization_id, video_id, source_version) ON DELETE RESTRICT,
  CHECK (substr(final_object_key, 1, length('tenants/' || organization_id || '/videos/' || video_id || '/derivatives/')) =
         'tenants/' || organization_id || '/videos/' || video_id || '/derivatives/'),
  CHECK (instr(final_object_key, '..') = 0 AND instr(final_object_key, char(92)) = 0
         AND instr(final_object_key, '?') = 0 AND instr(final_object_key, '#') = 0
         AND instr(final_object_key, '%') = 0),
  CHECK (staging_object_key IS NULL OR staging_object_key GLOB final_object_key || '.attempt-[1-9]*.partial'),
  CHECK ((lease_token_digest IS NULL) = (lease_expires_at_ms IS NULL)),
  CHECK ((staged_checksum_sha256 IS NULL) = (staged_bytes IS NULL)),
  CHECK ((state IN ('leased', 'transforming', 'staged', 'publishing')) = (lease_token_digest IS NOT NULL)),
  CHECK (state NOT IN ('staged', 'publishing', 'succeeded') OR staged_checksum_sha256 IS NOT NULL),
  CHECK ((state = 'succeeded') = (manifest_digest IS NOT NULL)),
  CHECK (failure_class IS NULL OR state IN (
    'cancelling', 'fallback_queued', 'failed', 'cancelled', 'dead_letter'
  )),
  CHECK (fallback_executor IS NULL OR selected_executor = 'cloudflare_media'),
  CHECK (updated_at_ms >= created_at_ms)
);
CREATE INDEX media_job_execution_v1_ready_idx
  ON media_job_execution_v1(state, selected_executor, updated_at_ms);
CREATE INDEX media_job_execution_v1_lease_idx
  ON media_job_execution_v1(state, lease_expires_at_ms);

CREATE TRIGGER media_job_execution_v1_authority
BEFORE INSERT ON media_job_execution_v1
WHEN NOT EXISTS (
  SELECT 1 FROM media_jobs j
   WHERE j.id = NEW.job_id AND j.organization_id = NEW.organization_id
     AND j.video_id = NEW.video_id AND j.source_version = NEW.source_version
     AND j.profile_version = NEW.profile_version AND j.output_object_key = NEW.final_object_key
) OR NOT EXISTS (
  SELECT 1 FROM media_profile_policies_v1 p
   WHERE p.profile_id = NEW.profile_id AND p.catalog_version = NEW.catalog_version
)
BEGIN
  SELECT RAISE(ABORT, 'media_job_execution_authority_mismatch');
END;

CREATE TRIGGER media_job_execution_v1_fence_monotonic
BEFORE UPDATE ON media_job_execution_v1
WHEN NEW.job_id <> OLD.job_id
  OR NEW.organization_id <> OLD.organization_id
  OR NEW.video_id <> OLD.video_id
  OR NEW.source_version <> OLD.source_version
  OR NEW.catalog_version <> OLD.catalog_version
  OR NEW.profile_id <> OLD.profile_id
  OR NEW.profile_version <> OLD.profile_version
  OR NEW.normalized_profile_sha256 <> OLD.normalized_profile_sha256
  OR NEW.final_object_key <> OLD.final_object_key
  OR NEW.output_content_type <> OLD.output_content_type
  OR NEW.max_output_bytes <> OLD.max_output_bytes
  OR NEW.attempt < OLD.attempt
  OR NEW.lease_epoch < OLD.lease_epoch
  OR NEW.provider_operations < OLD.provider_operations
  OR NEW.provider_output_seconds < OLD.provider_output_seconds
  OR NEW.native_cpu_millis < OLD.native_cpu_millis
  OR NEW.native_gpu_millis < OLD.native_gpu_millis
  OR NEW.scratch_byte_seconds < OLD.scratch_byte_seconds
  OR NEW.total_cost_microunits < OLD.total_cost_microunits
  OR NEW.updated_at_ms < OLD.updated_at_ms
BEGIN
  SELECT RAISE(ABORT, 'media_job_execution_regression');
END;

CREATE TRIGGER media_job_execution_v1_terminal
BEFORE UPDATE ON media_job_execution_v1
WHEN OLD.state IN ('succeeded', 'failed', 'cancelled', 'dead_letter')
BEGIN
  SELECT RAISE(ABORT, 'media_job_execution_terminal');
END;

CREATE TABLE media_output_manifests_v1 (
  manifest_digest TEXT PRIMARY KEY NOT NULL CHECK (
    length(manifest_digest) = 64 AND lower(manifest_digest) = manifest_digest
      AND manifest_digest NOT GLOB '*[^0-9a-f]*'
  ),
  job_id TEXT NOT NULL UNIQUE REFERENCES media_job_execution_v1(job_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  executor TEXT NOT NULL CHECK (executor IN (
    'cloudflare_media', 'native_gstreamer', 'external_provider', 'control_plane'
  )),
  source_checksum_sha256 TEXT NOT NULL CHECK (length(source_checksum_sha256) = 64),
  normalized_profile_sha256 TEXT NOT NULL CHECK (length(normalized_profile_sha256) = 64),
  object_key TEXT NOT NULL UNIQUE,
  object_checksum_sha256 TEXT NOT NULL CHECK (length(object_checksum_sha256) = 64),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  manifest_json TEXT NOT NULL CHECK (
    json_valid(manifest_json)
      AND json_extract(manifest_json, '$.schema_version') = 1
      AND json_extract(manifest_json, '$.job_id') = job_id
      AND json_extract(manifest_json, '$.executor') = executor
      AND json_extract(manifest_json, '$.source_checksum_sha256') = source_checksum_sha256
      AND json_extract(manifest_json, '$.normalized_profile_sha256') = normalized_profile_sha256
      AND json_extract(manifest_json, '$.object_key') = object_key
      AND json_extract(manifest_json, '$.object_checksum_sha256') = object_checksum_sha256
      AND json_extract(manifest_json, '$.bytes') = bytes
      AND json_extract(manifest_json, '$.content_type') = content_type
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (substr(object_key, 1, length('tenants/' || organization_id || '/videos/' || video_id || '/derivatives/')) =
         'tenants/' || organization_id || '/videos/' || video_id || '/derivatives/')
);

CREATE TRIGGER media_output_manifests_v1_authority
BEFORE INSERT ON media_output_manifests_v1
WHEN NOT EXISTS (
  SELECT 1 FROM media_job_execution_v1 e
  JOIN media_source_probes_v1 p
    ON p.organization_id = e.organization_id
   AND p.video_id = e.video_id
   AND p.source_version = e.source_version
  WHERE e.job_id = NEW.job_id
    AND e.organization_id = NEW.organization_id
    AND e.video_id = NEW.video_id
    AND e.selected_executor = NEW.executor
    AND e.final_object_key = NEW.object_key
    AND e.staged_checksum_sha256 = NEW.object_checksum_sha256
    AND e.staged_bytes = NEW.bytes
    AND e.output_content_type = NEW.content_type
    AND e.normalized_profile_sha256 = NEW.normalized_profile_sha256
    AND p.source_checksum_sha256 = NEW.source_checksum_sha256
    AND e.state = 'publishing'
    AND EXISTS (
      SELECT 1 FROM object_manifests om
       WHERE om.object_key = NEW.object_key
         AND om.organization_id = NEW.organization_id
         AND om.video_id = NEW.video_id
         AND om.checksum_sha256 = NEW.object_checksum_sha256
         AND om.bytes = NEW.bytes
         AND om.content_type = NEW.content_type
         AND om.state = 'available'
    )
    AND EXISTS (
      SELECT 1 FROM storage_governed_objects_v1 g
       WHERE g.organization_id = NEW.organization_id
         AND g.object_key = NEW.object_key
         AND g.checksum_sha256 = NEW.object_checksum_sha256
         AND g.bytes = NEW.bytes
         AND g.content_type = NEW.content_type
         AND g.visibility = 'private'
         AND g.state = 'active'
         AND g.malware_disposition = 'clean'
    )
)
BEGIN
  SELECT RAISE(ABORT, 'media_output_manifest_authority_mismatch');
END;
CREATE TRIGGER media_output_manifests_v1_no_update
BEFORE UPDATE ON media_output_manifests_v1
BEGIN SELECT RAISE(ABORT, 'media_output_manifest_immutable'); END;
CREATE TRIGGER media_output_manifests_v1_no_delete
BEFORE DELETE ON media_output_manifests_v1
BEGIN SELECT RAISE(ABORT, 'media_output_manifest_immutable'); END;

CREATE TABLE media_execution_events_v1 (
  job_id TEXT NOT NULL REFERENCES media_job_execution_v1(job_id) ON DELETE RESTRICT,
  event_sequence INTEGER NOT NULL CHECK (event_sequence BETWEEN 1 AND 9007199254740991),
  lease_epoch INTEGER NOT NULL CHECK (lease_epoch BETWEEN 0 AND 9007199254740991),
  state TEXT NOT NULL CHECK (state IN (
    'queued', 'leased', 'transforming', 'staged', 'publishing',
    'cancelling', 'fallback_queued', 'succeeded', 'failed', 'cancelled', 'dead_letter'
  )),
  progress_basis_points INTEGER CHECK (progress_basis_points BETWEEN 0 AND 10000),
  event_digest TEXT NOT NULL CHECK (
    length(event_digest) = 64 AND lower(event_digest) = event_digest
      AND event_digest NOT GLOB '*[^0-9a-f]*'
  ),
  previous_event_digest TEXT NOT NULL CHECK (
    length(previous_event_digest) = 64 AND lower(previous_event_digest) = previous_event_digest
      AND previous_event_digest NOT GLOB '*[^0-9a-f]*'
  ),
  safe_code TEXT CHECK (safe_code IS NULL OR (
    length(safe_code) BETWEEN 1 AND 64 AND safe_code = lower(safe_code)
      AND safe_code NOT GLOB '*[^a-z0-9_]*'
  )),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (job_id, event_sequence),
  UNIQUE (event_digest)
);

CREATE TRIGGER media_execution_events_v1_chain
BEFORE INSERT ON media_execution_events_v1
WHEN (NEW.event_sequence = 1 AND NEW.previous_event_digest <> printf('%064d', 0))
  OR (NEW.event_sequence > 1 AND NOT EXISTS (
    SELECT 1 FROM media_execution_events_v1 previous
     WHERE previous.job_id = NEW.job_id
       AND previous.event_sequence = NEW.event_sequence - 1
       AND previous.event_digest = NEW.previous_event_digest
  ))
BEGIN
  SELECT RAISE(ABORT, 'media_execution_event_chain_mismatch');
END;
CREATE TRIGGER media_execution_events_v1_no_update
BEFORE UPDATE ON media_execution_events_v1
BEGIN SELECT RAISE(ABORT, 'media_execution_event_immutable'); END;
CREATE TRIGGER media_execution_events_v1_no_delete
BEFORE DELETE ON media_execution_events_v1
BEGIN SELECT RAISE(ABORT, 'media_execution_event_immutable'); END;

-- Native workers stream only into an attempt-scoped staging object. This row
-- is persisted before the body is accepted, so cancellation/recovery can
-- identify an upload even when the Worker dies before acknowledging the PUT.
CREATE TABLE media_native_output_staging_v1 (
  job_id TEXT NOT NULL REFERENCES media_jobs(id) ON DELETE RESTRICT,
  attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 65535),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  worker_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  lease_token_digest TEXT NOT NULL CHECK (
    length(lease_token_digest) = 64 AND lower(lease_token_digest) = lease_token_digest
      AND lease_token_digest NOT GLOB '*[^0-9a-f]*'
  ),
  staging_object_key TEXT NOT NULL UNIQUE,
  final_object_key TEXT NOT NULL,
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64 AND lower(checksum_sha256) = checksum_sha256
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  state TEXT NOT NULL CHECK (state IN (
    'receiving', 'staged', 'published', 'cleaned', 'conflict'
  )),
  provider_etag TEXT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (job_id, attempt),
  CHECK (staging_object_key = final_object_key || '.attempt-' || attempt || '.' ||
         checksum_sha256 || '.partial'),
  CHECK (substr(final_object_key, 1, length(
    'tenants/' || organization_id || '/videos/' || video_id || '/derivatives/'
  )) = 'tenants/' || organization_id || '/videos/' || video_id || '/derivatives/'),
  CHECK (instr(staging_object_key, '..') = 0 AND instr(staging_object_key, char(92)) = 0
         AND instr(staging_object_key, '?') = 0 AND instr(staging_object_key, '#') = 0
         AND instr(staging_object_key, '%') = 0),
  CHECK ((state = 'receiving' AND provider_etag IS NULL)
         OR (state IN ('staged', 'published') AND provider_etag IS NOT NULL)
         OR state IN ('cleaned', 'conflict')),
  CHECK (updated_at_ms >= created_at_ms)
);
CREATE INDEX media_native_output_staging_v1_recovery_idx
  ON media_native_output_staging_v1(state, updated_at_ms, job_id);

CREATE TRIGGER media_native_output_staging_v1_authority
BEFORE INSERT ON media_native_output_staging_v1
WHEN NOT EXISTS (
  SELECT 1 FROM media_jobs j
   WHERE j.id = NEW.job_id AND j.organization_id = NEW.organization_id
     AND j.video_id = NEW.video_id AND j.output_object_key = NEW.final_object_key
     AND j.selected_executor = 'native_gstreamer' AND j.attempt = NEW.attempt
     AND j.worker_id = NEW.worker_id AND j.lease_token_digest = NEW.lease_token_digest
     AND j.state IN ('leased', 'running') AND j.cancel_requested = 0
)
BEGIN
  SELECT RAISE(ABORT, 'media_native_staging_authority_mismatch');
END;

CREATE TRIGGER media_native_output_staging_v1_transition
BEFORE UPDATE ON media_native_output_staging_v1
WHEN NEW.job_id <> OLD.job_id OR NEW.attempt <> OLD.attempt
  OR NEW.organization_id <> OLD.organization_id OR NEW.video_id <> OLD.video_id
  OR NEW.worker_id <> OLD.worker_id OR NEW.lease_token_digest <> OLD.lease_token_digest
  OR NEW.staging_object_key <> OLD.staging_object_key
  OR NEW.final_object_key <> OLD.final_object_key OR NEW.bytes <> OLD.bytes
  OR NEW.checksum_sha256 <> OLD.checksum_sha256 OR NEW.content_type <> OLD.content_type
  OR NEW.created_at_ms <> OLD.created_at_ms OR NEW.updated_at_ms < OLD.updated_at_ms
  OR OLD.state IN ('cleaned', 'conflict')
  OR (OLD.state = 'receiving' AND NEW.state NOT IN ('receiving', 'staged', 'cleaned', 'conflict'))
  OR (OLD.state = 'staged' AND NEW.state NOT IN ('staged', 'published', 'cleaned', 'conflict'))
  OR (OLD.state = 'published' AND NEW.state NOT IN ('published', 'cleaned', 'conflict'))
BEGIN
  SELECT RAISE(ABORT, 'media_native_staging_transition_invalid');
END;

CREATE TRIGGER media_native_output_staging_v1_no_delete
BEFORE DELETE ON media_native_output_staging_v1
BEGIN SELECT RAISE(ABORT, 'media_native_staging_immutable'); END;
