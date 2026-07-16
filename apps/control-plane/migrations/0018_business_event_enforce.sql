PRAGMA foreign_keys = ON;

-- Enforce initial event envelopes and storage-object authority in a bounded
-- D1 migration before ordered lifecycle transitions are installed.

CREATE TRIGGER outbox_scope_policy_v1
BEFORE INSERT ON outbox_events
WHEN NEW.organization_id IS NULL
  OR NEW.aggregate_type GLOB '*[^a-z0-9_.:]*'
  OR NEW.event_type GLOB '*[^a-z0-9_.:]*'
  OR NEW.payload_checksum IS NULL
  OR json_extract(NEW.payload_json, '$.schema_version') <> NEW.payload_schema_version
  OR NEW.state <> 'pending'
  OR NEW.attempt <> 0
  OR NEW.event_sequence <> 0
  OR NEW.event_fingerprint <> 'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9'
  OR NEW.revision <> 0
  OR NEW.lease_expires_at_ms IS NOT NULL
  OR NEW.delivered_at_ms IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'frame_business_document_invalid_v1');
END;

CREATE TRIGGER imported_videos_initial_contract_v1
BEFORE INSERT ON imported_videos
WHEN NEW.state <> 'queued'
  OR NEW.event_sequence <> 0
  OR NEW.event_fingerprint <> 'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9'
  OR NEW.revision <> 0
  OR NEW.error_class IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'frame_business_event_order_conflict_v1');
END;

CREATE TRIGGER video_uploads_initial_contract_v1
BEFORE INSERT ON video_uploads
WHEN NEW.state <> 'initiated'
  OR NEW.received_bytes <> 0
  OR NEW.checksum_sha256 IS NOT NULL
  OR NEW.event_sequence <> 0
  OR NEW.event_fingerprint <> 'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9'
  OR NEW.revision <> 0
BEGIN
  SELECT RAISE(ABORT, 'frame_business_event_order_conflict_v1');
END;

CREATE TRIGGER storage_objects_scope_policy_v1
BEFORE INSERT ON storage_objects
WHEN NEW.checksum_sha256 IS NULL OR NOT EXISTS (
  SELECT 1 FROM storage_integrations integration
  WHERE integration.id = NEW.integration_id
    AND integration.organization_id = NEW.organization_id
)
OR (
  NEW.video_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM videos video
    WHERE video.id = NEW.video_id AND video.organization_id = NEW.organization_id
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER storage_objects_update_scope_policy_v1
BEFORE UPDATE ON storage_objects
WHEN NEW.organization_id <> OLD.organization_id
  OR NEW.integration_id <> OLD.integration_id
  OR NEW.object_key <> OLD.object_key
  OR NEW.object_version <> OLD.object_version
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER business_derivative_manifests_scope_policy_v1
BEFORE INSERT ON business_derivative_manifests_v1
WHEN NOT EXISTS (
  SELECT 1 FROM storage_objects source
  WHERE source.id = NEW.source_object_id
    AND source.organization_id = NEW.organization_id
    AND source.object_version = NEW.source_version
)
OR (
  NEW.output_object_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM storage_objects output
    WHERE output.id = NEW.output_object_id
      AND output.organization_id = NEW.organization_id
      AND output.object_key = NEW.output_object_key
      AND output.checksum_sha256 = NEW.output_checksum
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;
