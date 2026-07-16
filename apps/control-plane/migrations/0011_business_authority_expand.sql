PRAGMA foreign_keys = ON;

-- Issue 15 expand migration. Existing 0003-0005 rows remain readable while
-- the v1 repository writes authority-fenced metadata and audit receipts.

ALTER TABLE videos ADD COLUMN metadata_schema_version INTEGER NOT NULL DEFAULT 1
  CHECK (metadata_schema_version BETWEEN 1 AND 65535);
ALTER TABLE videos ADD COLUMN metadata_checksum TEXT
  CHECK (metadata_checksum IS NULL OR (
    length(metadata_checksum) = 64 AND metadata_checksum NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE videos ADD COLUMN comments_enabled INTEGER NOT NULL DEFAULT 1
  CHECK (comments_enabled IN (0, 1));
ALTER TABLE videos ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE video_edits ADD COLUMN document_checksum TEXT
  CHECK (document_checksum IS NULL OR (
    length(document_checksum) = 64 AND document_checksum NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE video_edits ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE shared_videos ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE shared_videos ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE comments ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE comments ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE comments ADD COLUMN comment_kind TEXT NOT NULL DEFAULT 'text'
  CHECK (comment_kind IN ('text', 'emoji'));
ALTER TABLE comments ADD COLUMN timeline_micros INTEGER
  CHECK (timeline_micros IS NULL OR timeline_micros BETWEEN 0 AND 9007199254740991);
UPDATE comments
SET organization_id = (SELECT organization_id FROM videos WHERE videos.id = comments.video_id)
WHERE organization_id IS NULL;

ALTER TABLE notifications ADD COLUMN payload_schema_version INTEGER NOT NULL DEFAULT 1
  CHECK (payload_schema_version BETWEEN 1 AND 65535);
ALTER TABLE notifications ADD COLUMN payload_checksum TEXT
  CHECK (payload_checksum IS NULL OR (
    length(payload_checksum) = 64 AND payload_checksum NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE notifications ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE outbox_events ADD COLUMN event_sequence INTEGER NOT NULL DEFAULT 0
  CHECK (event_sequence BETWEEN 0 AND 9007199254740991);
ALTER TABLE outbox_events ADD COLUMN event_fingerprint TEXT
  CHECK (event_fingerprint IS NULL OR (
    length(event_fingerprint) = 64 AND event_fingerprint NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE outbox_events ADD COLUMN payload_schema_version INTEGER NOT NULL DEFAULT 1
  CHECK (payload_schema_version BETWEEN 1 AND 65535);
ALTER TABLE outbox_events ADD COLUMN payload_checksum TEXT
  CHECK (payload_checksum IS NULL OR (
    length(payload_checksum) = 64 AND payload_checksum NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE outbox_events ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE outbox_events ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE video_uploads ADD COLUMN event_sequence INTEGER NOT NULL DEFAULT 0
  CHECK (event_sequence BETWEEN 0 AND 9007199254740991);
ALTER TABLE video_uploads ADD COLUMN event_fingerprint TEXT
  CHECK (event_fingerprint IS NULL OR (
    length(event_fingerprint) = 64 AND event_fingerprint NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE video_uploads ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE storage_integrations ADD COLUMN authority_version INTEGER NOT NULL DEFAULT 0
  CHECK (authority_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE storage_integrations ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE storage_integrations ADD COLUMN capabilities_schema_version INTEGER NOT NULL DEFAULT 1
  CHECK (capabilities_schema_version BETWEEN 1 AND 65535);
ALTER TABLE storage_integrations ADD COLUMN capabilities_checksum TEXT
  CHECK (capabilities_checksum IS NULL OR (
    length(capabilities_checksum) = 64 AND capabilities_checksum NOT GLOB '*[^0-9a-f]*'
  ));

ALTER TABLE storage_objects ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0
  CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991);
ALTER TABLE storage_objects ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE storage_objects ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
UPDATE storage_objects SET updated_at_ms = created_at_ms WHERE updated_at_ms = 0;

ALTER TABLE imported_videos ADD COLUMN event_sequence INTEGER NOT NULL DEFAULT 0
  CHECK (event_sequence BETWEEN 0 AND 9007199254740991);
ALTER TABLE imported_videos ADD COLUMN event_fingerprint TEXT
  CHECK (event_fingerprint IS NULL OR (
    length(event_fingerprint) = 64 AND event_fingerprint NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE imported_videos ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE imported_videos ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE developer_apps ADD COLUMN authority_version INTEGER NOT NULL DEFAULT 0
  CHECK (authority_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE developer_apps ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE developer_app_domains ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE developer_app_domains ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE developer_api_keys ADD COLUMN display_prefix TEXT
  CHECK (display_prefix IS NULL OR length(display_prefix) BETWEEN 4 AND 12);
ALTER TABLE developer_api_keys ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE developer_api_keys ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE developer_videos ADD COLUMN metadata_schema_version INTEGER NOT NULL DEFAULT 1
  CHECK (metadata_schema_version BETWEEN 1 AND 65535);
ALTER TABLE developer_videos ADD COLUMN metadata_checksum TEXT
  CHECK (metadata_checksum IS NULL OR (
    length(metadata_checksum) = 64 AND metadata_checksum NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE developer_videos ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE developer_videos ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE developer_videos ADD COLUMN external_user_digest TEXT
  CHECK (external_user_digest IS NULL OR (
    length(external_user_digest) = 64 AND external_user_digest NOT GLOB '*[^0-9a-f]*'
  ));

ALTER TABLE developer_credit_accounts ADD COLUMN ledger_sequence INTEGER NOT NULL DEFAULT 0
  CHECK (ledger_sequence BETWEEN 0 AND 9007199254740991);
ALTER TABLE developer_credit_accounts ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);
ALTER TABLE developer_credit_transactions ADD COLUMN ledger_sequence INTEGER
  CHECK (ledger_sequence IS NULL OR ledger_sequence BETWEEN 1 AND 9007199254740991);
ALTER TABLE developer_credit_transactions ADD COLUMN reference_digest TEXT
  CHECK (reference_digest IS NULL OR (
    length(reference_digest) = 64 AND reference_digest NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE developer_credit_transactions ADD COLUMN operation_id TEXT
  CHECK (operation_id IS NULL OR length(operation_id) = 36);
ALTER TABLE developer_credit_transactions ADD COLUMN request_fingerprint TEXT
  CHECK (request_fingerprint IS NULL OR (
    length(request_fingerprint) = 64 AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ));

ALTER TABLE usage_ledger ADD COLUMN operation_id TEXT
  CHECK (operation_id IS NULL OR length(operation_id) = 36);
ALTER TABLE usage_ledger ADD COLUMN request_fingerprint TEXT
  CHECK (request_fingerprint IS NULL OR (
    length(request_fingerprint) = 64 AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE developer_daily_storage_snapshots ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE developer_daily_storage_snapshots ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

-- The aggregate repository's v0.8 transient title envelope predates the
-- ordered/checksummed business outbox columns added above. Extend that empty
-- envelope here; migration 0021 replaces its trigger after the larger
-- authority migrations, keeping each D1 migration inside parser limits.
ALTER TABLE repository_video_title_operations ADD COLUMN payload_checksum TEXT
  CHECK (payload_checksum IS NULL OR (
    length(payload_checksum) = 64 AND payload_checksum NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE repository_video_title_operations ADD COLUMN event_fingerprint TEXT
  CHECK (event_fingerprint IS NULL OR (
    length(event_fingerprint) = 64 AND event_fingerprint NOT GLOB '*[^0-9a-f]*'
  ));

CREATE INDEX comments_org_video_active_idx
  ON comments(organization_id, video_id, created_at_ms, id) WHERE deleted_at_ms IS NULL;
CREATE INDEX outbox_events_org_delivery_v1_idx
  ON outbox_events(organization_id, state, available_at_ms, event_sequence, id);
CREATE INDEX imported_videos_org_event_v1_idx
  ON imported_videos(organization_id, state, event_sequence, id);
CREATE INDEX storage_objects_org_video_role_v1_idx
  ON storage_objects(organization_id, video_id, role, object_version, state);
CREATE INDEX developer_apps_org_status_v1_idx
  ON developer_apps(organization_id, status, id);
CREATE UNIQUE INDEX developer_credit_transactions_account_sequence_v1_idx
  ON developer_credit_transactions(account_id, ledger_sequence)
  WHERE ledger_sequence IS NOT NULL;
CREATE UNIQUE INDEX developer_credit_transactions_operation_v1_idx
  ON developer_credit_transactions(operation_id) WHERE operation_id IS NOT NULL;
CREATE UNIQUE INDEX usage_ledger_operation_v1_idx
  ON usage_ledger(operation_id) WHERE operation_id IS NOT NULL;

CREATE TABLE business_repository_assertions_v1 (
  id TEXT PRIMARY KEY NOT NULL,
  satisfied INTEGER NOT NULL CHECK (satisfied = 1)
);
CREATE TRIGGER business_repository_assertions_v1_conflict
BEFORE INSERT ON business_repository_assertions_v1
WHEN NEW.satisfied <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TABLE business_retention_assertions_v1 (
  id TEXT PRIMARY KEY NOT NULL,
  satisfied INTEGER NOT NULL CHECK (satisfied = 1)
);
CREATE TRIGGER business_retention_assertions_v1_locked
BEFORE INSERT ON business_retention_assertions_v1
WHEN NEW.satisfied <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_business_retention_locked_v1');
END;

CREATE TABLE business_repository_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  principal_kind TEXT NOT NULL CHECK (principal_kind IN ('user', 'anonymous')),
  principal_subject TEXT NOT NULL CHECK (
    (principal_kind = 'user' AND length(principal_subject) = 36)
    OR (principal_kind = 'anonymous' AND length(principal_subject) = 64
        AND principal_subject NOT GLOB '*[^0-9a-f]*')
  ),
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 128),
  action TEXT NOT NULL CHECK (action IN (
    'video_manage', 'edit_manage', 'share_manage', 'comment_create', 'comment_delete',
    'notification_manage', 'storage_manage', 'import_manage', 'developer_manage',
    'ledger_manage', 'data_export', 'data_delete'
  )),
  subject_id TEXT NOT NULL CHECK (length(subject_id) BETWEEN 1 AND 255),
  request_fingerprint TEXT NOT NULL CHECK (
    length(request_fingerprint) = 64 AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  result_code TEXT NOT NULL CHECK (result_code IN (
    'created', 'applied', 'accepted', 'revoked', 'tombstoned', 'purged', 'unchanged'
  )),
  resulting_revision INTEGER NOT NULL CHECK (resulting_revision BETWEEN 0 AND 9007199254740991),
  committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (organization_id, principal_kind, principal_subject, idempotency_key)
);
CREATE INDEX business_repository_operations_v1_subject_idx
  ON business_repository_operations_v1(organization_id, action, subject_id);

CREATE TRIGGER business_repository_operations_v1_immutable_update
BEFORE UPDATE ON business_repository_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'business operation receipts are immutable');
END;
CREATE TRIGGER business_repository_operations_v1_immutable_delete
BEFORE DELETE ON business_repository_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'business operation receipts are immutable');
END;

CREATE TABLE business_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  principal_kind TEXT NOT NULL CHECK (principal_kind IN ('user', 'anonymous')),
  principal_subject_digest TEXT NOT NULL CHECK (
    length(principal_subject_digest) = 64 AND principal_subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  action TEXT NOT NULL CHECK (length(action) BETWEEN 3 AND 64),
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64 AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'error')),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX business_audit_events_v1_org_time_idx
  ON business_audit_events_v1(organization_id, occurred_at_ms, id);
CREATE TRIGGER business_audit_events_v1_immutable_update
BEFORE UPDATE ON business_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'business audit events are immutable');
END;
CREATE TRIGGER business_audit_events_v1_immutable_delete
BEFORE DELETE ON business_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'business audit events are immutable');
END;

CREATE TABLE business_event_inbox_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  aggregate_kind TEXT NOT NULL CHECK (aggregate_kind IN ('outbox', 'import', 'upload')),
  aggregate_id TEXT NOT NULL,
  event_sequence INTEGER NOT NULL CHECK (event_sequence BETWEEN 1 AND 9007199254740991),
  event_fingerprint TEXT NOT NULL CHECK (
    length(event_fingerprint) = 64 AND event_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  target_state TEXT NOT NULL CHECK (length(target_state) BETWEEN 3 AND 32),
  disposition TEXT NOT NULL CHECK (disposition IN ('deferred', 'applied', 'stale')),
  received_at_ms INTEGER NOT NULL CHECK (received_at_ms BETWEEN 0 AND 9007199254740991),
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  PRIMARY KEY (organization_id, aggregate_kind, aggregate_id, event_sequence)
);
CREATE INDEX business_event_inbox_v1_ready_idx
  ON business_event_inbox_v1(organization_id, aggregate_kind, aggregate_id, disposition, event_sequence);
CREATE TRIGGER business_event_inbox_v1_fingerprint_guard
BEFORE INSERT ON business_event_inbox_v1
WHEN EXISTS (
  SELECT 1 FROM business_event_inbox_v1 existing
  WHERE existing.organization_id = NEW.organization_id
    AND existing.aggregate_kind = NEW.aggregate_kind
    AND existing.aggregate_id = NEW.aggregate_id
    AND existing.event_sequence = NEW.event_sequence
    AND existing.event_fingerprint <> NEW.event_fingerprint
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_semantic_replay_conflict_v1');
END;

CREATE TABLE business_derivative_manifests_v1 (
  job_id TEXT PRIMARY KEY NOT NULL REFERENCES media_jobs(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  executor TEXT NOT NULL CHECK (executor IN ('cloudflare_media', 'native_gstreamer')),
  source_object_id TEXT NOT NULL REFERENCES storage_objects(id) ON DELETE RESTRICT,
  source_version INTEGER NOT NULL CHECK (source_version BETWEEN 1 AND 9007199254740991),
  transform_profile TEXT NOT NULL CHECK (length(transform_profile) BETWEEN 1 AND 128),
  profile_version INTEGER NOT NULL CHECK (profile_version BETWEEN 1 AND 9007199254740991),
  output_role TEXT NOT NULL CHECK (output_role IN (
    'source', 'segment', 'thumbnail', 'preview', 'spritesheet', 'audio', 'export', 'manifest'
  )),
  output_object_id TEXT REFERENCES storage_objects(id) ON DELETE SET NULL,
  output_object_key TEXT NOT NULL CHECK (length(output_object_key) BETWEEN 1 AND 1024),
  output_checksum TEXT CHECK (output_checksum IS NULL OR (
    length(output_checksum) = 64 AND output_checksum NOT GLOB '*[^0-9a-f]*'
  )),
  output_content_type TEXT NOT NULL CHECK (length(output_content_type) BETWEEN 3 AND 127),
  state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
  usage_units INTEGER NOT NULL DEFAULT 0 CHECK (usage_units BETWEEN 0 AND 9007199254740991),
  cost_microcredits INTEGER NOT NULL DEFAULT 0 CHECK (cost_microcredits BETWEEN 0 AND 9007199254740991),
  failure_class TEXT CHECK (
    failure_class IS NULL OR (
      length(failure_class) BETWEEN 1 AND 64 AND failure_class NOT GLOB '*[^a-z0-9_]*'
    )
  ),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK ((state = 'succeeded') = (output_object_id IS NOT NULL AND output_checksum IS NOT NULL)),
  CHECK ((state = 'failed') = (failure_class IS NOT NULL))
);
CREATE INDEX business_derivative_manifests_v1_org_state_idx
  ON business_derivative_manifests_v1(organization_id, state, job_id);

CREATE TABLE business_data_handling_policies_v1 (
  data_class TEXT PRIMARY KEY NOT NULL,
  exportable INTEGER NOT NULL CHECK (exportable IN (0, 1)),
  deletion_mode TEXT NOT NULL CHECK (deletion_mode IN (
    'tombstone_then_purge', 'cryptographic_erase_then_purge',
    'append_compensating_entry', 'retain_audit_only', 'excluded_quarantine'
  )),
  default_retention_days INTEGER NOT NULL CHECK (default_retention_days BETWEEN 1 AND 36500),
  legal_hold_supported INTEGER NOT NULL CHECK (legal_hold_supported IN (0, 1))
);
INSERT INTO business_data_handling_policies_v1 VALUES
  ('video_metadata',1,'tombstone_then_purge',30,1),
  ('video_edit',1,'tombstone_then_purge',30,1),
  ('share',1,'tombstone_then_purge',30,1),
  ('comment',1,'tombstone_then_purge',30,1),
  ('notification',1,'tombstone_then_purge',90,0),
  ('outbox',1,'tombstone_then_purge',90,0),
  ('storage_integration',0,'cryptographic_erase_then_purge',30,0),
  ('storage_object',1,'tombstone_then_purge',30,1),
  ('derivative_job',1,'tombstone_then_purge',30,1),
  ('upload',1,'tombstone_then_purge',30,1),
  ('import',1,'tombstone_then_purge',30,1),
  ('developer_app',1,'tombstone_then_purge',30,1),
  ('developer_domain',1,'tombstone_then_purge',30,1),
  ('developer_api_key',0,'cryptographic_erase_then_purge',30,0),
  ('developer_video',1,'tombstone_then_purge',30,1),
  ('credit_account',1,'retain_audit_only',2555,1),
  ('credit_transaction',1,'append_compensating_entry',2555,1),
  ('usage_ledger',1,'append_compensating_entry',2555,1),
  ('daily_storage_snapshot',1,'retain_audit_only',2555,1),
  ('messenger_legacy',0,'excluded_quarantine',30,0);

CREATE TABLE business_legal_holds_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  data_class TEXT NOT NULL REFERENCES business_data_handling_policies_v1(data_class) ON DELETE RESTRICT,
  subject_id TEXT NOT NULL,
  reason_code TEXT NOT NULL CHECK (
    length(reason_code) BETWEEN 1 AND 64 AND reason_code NOT GLOB '*[^a-z0-9_]*'
  ),
  placed_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  placed_at_ms INTEGER NOT NULL CHECK (placed_at_ms BETWEEN 0 AND 9007199254740991),
  released_at_ms INTEGER CHECK (released_at_ms IS NULL OR released_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX business_legal_holds_v1_active_idx
  ON business_legal_holds_v1(organization_id, data_class, subject_id) WHERE released_at_ms IS NULL;

CREATE TABLE business_data_requests_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  data_class TEXT NOT NULL REFERENCES business_data_handling_policies_v1(data_class) ON DELETE RESTRICT,
  subject_id TEXT NOT NULL,
  request_kind TEXT NOT NULL CHECK (request_kind IN ('export', 'delete')),
  disposition TEXT NOT NULL CHECK (disposition IN (
    'scheduled', 'blocked_by_hold', 'completed', 'failed', 'compensated', 'quarantined'
  )),
  manifest_checksum TEXT CHECK (manifest_checksum IS NULL OR (
    length(manifest_checksum) = 64 AND manifest_checksum NOT GLOB '*[^0-9a-f]*'
  )),
  requested_at_ms INTEGER NOT NULL CHECK (requested_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991),
  operation_id TEXT NOT NULL UNIQUE CHECK (length(operation_id) = 36)
);
CREATE INDEX business_data_requests_v1_org_state_idx
  ON business_data_requests_v1(organization_id, disposition, requested_at_ms);

CREATE TABLE business_source_table_map_v1 (
  source_table TEXT PRIMARY KEY NOT NULL,
  aggregate TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN ('retained', 'excluded_fail_closed')),
  target_contract TEXT NOT NULL
);
INSERT INTO business_source_table_map_v1 VALUES
  ('videos','media_metadata','retained','videos+business_repository_operations_v1'),
  ('video_edits','media_edits','retained','video_edits'),
  ('shared_videos','sharing','retained','shared_videos'),
  ('comments','collaboration','retained','comments'),
  ('notifications','notification','retained','notifications+outbox_events'),
  ('messenger_conversations','messenger','excluded_fail_closed','business_messenger_legacy_quarantine_v1'),
  ('messenger_messages','messenger','excluded_fail_closed','business_messenger_legacy_quarantine_v1'),
  ('messenger_support_emails','messenger','excluded_fail_closed','business_messenger_legacy_quarantine_v1'),
  ('s3_buckets','storage','retained','storage_integrations'),
  ('storage_integrations','storage','retained','storage_integrations'),
  ('storage_objects','storage','retained','storage_objects'),
  ('video_uploads','uploads','retained','video_uploads'),
  ('imported_videos','imports','retained','imported_videos+business_event_inbox_v1'),
  ('developer_apps','developer','retained','developer_apps'),
  ('developer_app_domains','developer','retained','developer_app_domains'),
  ('developer_api_keys','developer','retained','developer_api_keys'),
  ('developer_videos','developer','retained','developer_videos'),
  ('developer_credit_accounts','billing','retained','developer_credit_accounts'),
  ('developer_credit_transactions','billing','retained','developer_credit_transactions'),
  ('developer_daily_storage_snapshots','billing','retained','developer_daily_storage_snapshots');

CREATE TABLE business_derived_aggregate_map_v1 (
  aggregate TEXT PRIMARY KEY NOT NULL,
  provenance TEXT NOT NULL CHECK (provenance IN ('frame_derived')),
  rationale TEXT NOT NULL
);
INSERT INTO business_derived_aggregate_map_v1 VALUES
  ('usage_ledger','frame_derived','Frame auditable usage facts; absent from pinned Cap schema');

CREATE TABLE business_messenger_legacy_quarantine_v1 (
  source_table TEXT NOT NULL,
  source_id TEXT NOT NULL,
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  disposition TEXT NOT NULL DEFAULT 'quarantined' CHECK (disposition IN ('quarantined', 'purged')),
  quarantined_at_ms INTEGER NOT NULL CHECK (quarantined_at_ms BETWEEN 0 AND 9007199254740991),
  purge_after_ms INTEGER NOT NULL CHECK (purge_after_ms BETWEEN 0 AND 9007199254740991),
  purged_at_ms INTEGER CHECK (purged_at_ms IS NULL OR purged_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36),
  PRIMARY KEY (source_table, source_id),
  CHECK (purge_after_ms > quarantined_at_ms),
  CHECK ((disposition = 'purged') = (purged_at_ms IS NOT NULL AND last_operation_id IS NOT NULL))
);
INSERT OR IGNORE INTO business_messenger_legacy_quarantine_v1
  (source_table, source_id, organization_id, disposition, quarantined_at_ms, purge_after_ms)
SELECT 'messenger_conversations', conversation.id,
       (SELECT MIN(membership.organization_id)
        FROM organization_members membership
        WHERE membership.user_id = conversation.user_id AND membership.state = 'active'
        HAVING COUNT(DISTINCT membership.organization_id) = 1),
       'quarantined',
       CAST(unixepoch('now') AS INTEGER) * 1000,
       (CAST(unixepoch('now') AS INTEGER) + 2592000) * 1000
FROM messenger_conversations conversation;
INSERT OR IGNORE INTO business_messenger_legacy_quarantine_v1
  (source_table, source_id, organization_id, disposition, quarantined_at_ms, purge_after_ms)
SELECT 'messenger_messages', message.id,
       (SELECT MIN(membership.organization_id)
        FROM messenger_conversations conversation
        JOIN organization_members membership ON membership.user_id = conversation.user_id
        WHERE conversation.id = message.conversation_id AND membership.state = 'active'
        HAVING COUNT(DISTINCT membership.organization_id) = 1),
       'quarantined',
       CAST(unixepoch('now') AS INTEGER) * 1000,
       (CAST(unixepoch('now') AS INTEGER) + 2592000) * 1000
FROM messenger_messages message;
INSERT OR IGNORE INTO business_messenger_legacy_quarantine_v1
  (source_table, source_id, organization_id, disposition, quarantined_at_ms, purge_after_ms)
SELECT 'messenger_support_emails', email.id,
       (SELECT MIN(membership.organization_id)
        FROM messenger_conversations conversation
        JOIN organization_members membership
          ON membership.user_id = COALESCE(email.user_id, conversation.user_id)
        WHERE conversation.id = email.conversation_id AND membership.state = 'active'
        HAVING COUNT(DISTINCT membership.organization_id) = 1),
       'quarantined',
       CAST(unixepoch('now') AS INTEGER) * 1000,
       (CAST(unixepoch('now') AS INTEGER) + 2592000) * 1000
FROM messenger_support_emails email;

CREATE INDEX business_messenger_legacy_quarantine_scope_v1_idx
  ON business_messenger_legacy_quarantine_v1(organization_id, disposition, purge_after_ms);
