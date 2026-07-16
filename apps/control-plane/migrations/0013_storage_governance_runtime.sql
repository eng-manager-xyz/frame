PRAGMA foreign_keys = ON;

-- Durable, tenant-first authority for every object path. Provider listings are
-- deliberately absent from this schema: they are observations, never authority.
CREATE TABLE storage_governed_objects_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN (
    'source', 'recording_segment', 'thumbnail', 'preview', 'spritesheet', 'audio',
    'export', 'caption', 'avatar', 'manifest', 'multipart_session', 'backup_copy'
  )),
  visibility TEXT NOT NULL CHECK (visibility IN ('private', 'unlisted', 'public')),
  state TEXT NOT NULL CHECK (state IN ('active', 'quarantined', 'tombstoned', 'erased')),
  malware_disposition TEXT NOT NULL CHECK (malware_disposition IN ('pending', 'clean', 'rejected')),
  immutable_revision INTEGER NOT NULL CHECK (immutable_revision BETWEEN 1 AND 9007199254740991),
  cache_generation INTEGER NOT NULL CHECK (cache_generation BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64 AND lower(checksum_sha256) = checksum_sha256
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  retention_until_ms INTEGER CHECK (
    retention_until_ms IS NULL OR retention_until_ms BETWEEN 0 AND 9007199254740991
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, object_key),
  CHECK (substr(object_key, 1, length('tenants/' || organization_id || '/')) =
         'tenants/' || organization_id || '/'),
  CHECK (instr(object_key, '..') = 0 AND instr(object_key, char(92)) = 0
         AND instr(object_key, '?') = 0 AND instr(object_key, '#') = 0
         AND instr(object_key, '%') = 0),
  CHECK (state <> 'active' OR malware_disposition <> 'rejected')
);
CREATE INDEX storage_governed_objects_v1_state_idx
  ON storage_governed_objects_v1(organization_id, state, updated_at_ms);

-- Bridge existing authoritative object manifests into the governed authority.
-- Unsupported legacy roles fail closed and are intentionally not invented.
INSERT OR IGNORE INTO storage_governed_objects_v1(
  organization_id, object_key, role, visibility, state, malware_disposition,
  immutable_revision, cache_generation, checksum_sha256, bytes, content_type,
  retention_until_ms, created_at_ms, updated_at_ms
)
SELECT om.organization_id,
       om.object_key,
       CASE om.role
         WHEN 'source' THEN 'source'
         WHEN 'segment' THEN 'recording_segment'
         WHEN 'thumbnail' THEN 'thumbnail'
         WHEN 'preview' THEN 'preview'
         WHEN 'spritesheet' THEN 'spritesheet'
         WHEN 'audio' THEN 'audio'
         WHEN 'export' THEN 'export'
         WHEN 'manifest' THEN 'manifest'
       END,
       COALESCE(v.privacy, 'private'),
       CASE om.state
         WHEN 'available' THEN 'active'
         WHEN 'quarantined' THEN 'quarantined'
         WHEN 'deleting' THEN 'tombstoned'
         WHEN 'deleted' THEN 'erased'
         WHEN 'missing' THEN 'erased'
         ELSE 'quarantined'
       END,
       CASE om.state WHEN 'available' THEN 'clean' ELSE 'pending' END,
       om.object_version,
       CASE WHEN v.revision BETWEEN 1 AND 9007199254740991 THEN v.revision ELSE 1 END,
       om.checksum_sha256,
       om.bytes,
       om.content_type,
       NULL,
       om.created_at_ms,
       COALESCE(om.updated_at_ms, om.created_at_ms)
FROM object_manifests om
LEFT JOIN videos v ON v.id = om.video_id AND v.organization_id = om.organization_id
WHERE om.organization_id IS NOT NULL
  AND om.role IN ('source','segment','thumbnail','preview','spritesheet','audio','export','manifest')
  AND om.object_version BETWEEN 1 AND 9007199254740991
  AND om.bytes BETWEEN 1 AND 9007199254740991
  AND om.checksum_sha256 IS NOT NULL
  AND length(om.checksum_sha256) = 64
  AND lower(om.checksum_sha256) = om.checksum_sha256
  AND om.checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  AND substr(om.object_key, 1, length('tenants/' || om.organization_id || '/')) =
      'tenants/' || om.organization_id || '/'
  AND instr(om.object_key, '..') = 0 AND instr(om.object_key, char(92)) = 0
  AND instr(om.object_key, '?') = 0 AND instr(om.object_key, '#') = 0
  AND instr(om.object_key, '%') = 0;

CREATE TABLE storage_signed_grants_v1 (
  grant_id TEXT PRIMARY KEY NOT NULL CHECK (length(grant_id) = 36),
  organization_id TEXT NOT NULL,
  object_key TEXT NOT NULL,
  key_version INTEGER NOT NULL CHECK (key_version BETWEEN 1 AND 9007199254740991),
  operation TEXT NOT NULL CHECK (operation IN ('read', 'read_range')),
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 1 AND 9007199254740991),
  nonce_digest TEXT NOT NULL CHECK (
    length(nonce_digest) = 64 AND lower(nonce_digest) = nonce_digest
      AND nonce_digest NOT GLOB '*[^0-9a-f]*'
  ),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  grant_json TEXT NOT NULL CHECK (
    json_valid(grant_json)
      AND json_extract(grant_json, '$.schema_version') = 1
      AND json_extract(grant_json, '$.grant_id') = grant_id
      AND json_extract(grant_json, '$.key_version') = key_version
      AND json_extract(grant_json, '$.tenant_id') = organization_id
      AND json_extract(grant_json, '$.object_id') = object_key
      AND json_extract(grant_json, '$.operation') = operation
      AND json_extract(grant_json, '$.issued_at') = issued_at_ms
      AND json_extract(grant_json, '$.expires_at') = expires_at_ms
      AND json_extract(grant_json, '$.nonce_digest') = nonce_digest
      AND (
        (revoked_at_ms IS NULL AND json_type(grant_json, '$.revoked_at') = 'null')
        OR json_extract(grant_json, '$.revoked_at') = revoked_at_ms
      )
  ),
  FOREIGN KEY (organization_id, object_key)
    REFERENCES storage_governed_objects_v1(organization_id, object_key) ON DELETE RESTRICT,
  CHECK (expires_at_ms > issued_at_ms AND expires_at_ms - issued_at_ms <= 900000),
  CHECK (revoked_at_ms IS NULL OR revoked_at_ms >= issued_at_ms)
);
CREATE INDEX storage_signed_grants_v1_lookup_idx
  ON storage_signed_grants_v1(organization_id, grant_id, expires_at_ms);
CREATE TRIGGER storage_signed_grants_v1_exact_object
BEFORE INSERT ON storage_signed_grants_v1
WHEN NOT EXISTS (
  SELECT 1 FROM storage_governed_objects_v1 g
   WHERE g.organization_id = NEW.organization_id
     AND g.object_key = NEW.object_key
     AND g.immutable_revision = json_extract(NEW.grant_json, '$.immutable_revision')
     AND g.cache_generation = json_extract(NEW.grant_json, '$.cache_generation')
     AND g.checksum_sha256 = json_extract(NEW.grant_json, '$.object_checksum')
     AND g.state = 'active'
)
BEGIN
  SELECT RAISE(ABORT, 'storage_grant_object_mismatch');
END;
CREATE TRIGGER storage_signed_grants_v1_no_delete
BEFORE DELETE ON storage_signed_grants_v1
BEGIN
  SELECT RAISE(ABORT, 'storage_grants_are_durable');
END;
CREATE TRIGGER storage_signed_grants_v1_revoke_only
BEFORE UPDATE ON storage_signed_grants_v1
WHEN OLD.revoked_at_ms IS NOT NULL
  OR NEW.revoked_at_ms IS NULL
  OR NEW.revoked_at_ms < OLD.issued_at_ms
  OR NEW.grant_id <> OLD.grant_id
  OR NEW.organization_id <> OLD.organization_id
  OR NEW.object_key <> OLD.object_key
  OR NEW.key_version <> OLD.key_version
  OR NEW.operation <> OLD.operation
  OR NEW.issued_at_ms <> OLD.issued_at_ms
  OR NEW.expires_at_ms <> OLD.expires_at_ms
  OR NEW.nonce_digest <> OLD.nonce_digest
BEGIN
  SELECT RAISE(ABORT, 'storage_grant_update_forbidden');
END;

-- Custom-domain verification is a durable challenge -> active -> revoked workflow.
CREATE TABLE storage_custom_domains_v1 (
  domain_ascii TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  verification_version INTEGER NOT NULL CHECK (verification_version BETWEEN 1 AND 9007199254740991),
  challenge_digest TEXT NOT NULL CHECK (
    length(challenge_digest) = 64 AND lower(challenge_digest) = challenge_digest
      AND challenge_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'active', 'revoked')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  verified_at_ms INTEGER CHECK (verified_at_ms IS NULL OR verified_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  provider_receipt_digest TEXT CHECK (
    provider_receipt_digest IS NULL OR (
      length(provider_receipt_digest) = 64 AND lower(provider_receipt_digest) = provider_receipt_digest
        AND provider_receipt_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  CHECK (domain_ascii = lower(domain_ascii) AND length(domain_ascii) BETWEEN 3 AND 253),
  CHECK ((state = 'pending') = (verified_at_ms IS NULL AND revoked_at_ms IS NULL
                                AND provider_receipt_digest IS NULL)),
  CHECK ((state = 'active') = (verified_at_ms IS NOT NULL AND revoked_at_ms IS NULL
                               AND provider_receipt_digest IS NOT NULL)),
  CHECK ((state = 'revoked') = (verified_at_ms IS NOT NULL AND revoked_at_ms IS NOT NULL
                                AND provider_receipt_digest IS NOT NULL))
);
CREATE INDEX storage_custom_domains_v1_tenant_idx
  ON storage_custom_domains_v1(organization_id, state, domain_ascii);

CREATE TABLE storage_lifecycle_manifests_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64 AND lower(subject_digest) = subject_digest
      AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  manifest_digest TEXT NOT NULL CHECK (
    length(manifest_digest) = 64 AND lower(manifest_digest) = manifest_digest
      AND manifest_digest NOT GLOB '*[^0-9a-f]*'
  ),
  authority_revision INTEGER NOT NULL CHECK (authority_revision BETWEEN 1 AND 9007199254740991),
  retention_until_ms INTEGER CHECK (
    retention_until_ms IS NULL OR retention_until_ms BETWEEN 0 AND 9007199254740991
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, subject_digest)
);

CREATE TABLE storage_lifecycle_manifest_objects_v1 (
  organization_id TEXT NOT NULL,
  subject_digest TEXT NOT NULL,
  object_key TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN (
    'source', 'recording_segment', 'thumbnail', 'preview', 'spritesheet', 'audio',
    'export', 'caption', 'avatar', 'manifest', 'multipart_session', 'backup_copy'
  )),
  checksum_sha256 TEXT NOT NULL CHECK (
    length(checksum_sha256) = 64 AND lower(checksum_sha256) = checksum_sha256
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  retention_until_ms INTEGER CHECK (
    retention_until_ms IS NULL OR retention_until_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY (organization_id, subject_digest, object_key),
  UNIQUE (organization_id, subject_digest, role),
  FOREIGN KEY (organization_id, subject_digest)
    REFERENCES storage_lifecycle_manifests_v1(organization_id, subject_digest) ON DELETE RESTRICT,
  FOREIGN KEY (organization_id, object_key)
    REFERENCES storage_governed_objects_v1(organization_id, object_key) ON DELETE RESTRICT
);
CREATE INDEX storage_lifecycle_manifest_objects_v1_role_idx
  ON storage_lifecycle_manifest_objects_v1(organization_id, subject_digest, role);

-- Existing per-object and business holds are both authoritative blockers.
CREATE VIEW storage_active_hold_bridge_v1 AS
SELECT DISTINCT lm.organization_id, lm.subject_digest, 'object_legal_hold' AS authority
FROM storage_lifecycle_manifest_objects_v1 lm
JOIN storage_objects so
  ON so.organization_id = lm.organization_id AND so.object_key = lm.object_key
JOIN object_legal_holds oh ON oh.storage_object_id = so.id
WHERE oh.released_at_ms IS NULL
UNION
SELECT DISTINCT lm.organization_id, lm.subject_digest, 'business_legal_hold' AS authority
FROM storage_lifecycle_manifests_v1 lm
JOIN business_legal_holds_v1 bh
  ON bh.organization_id = lm.organization_id
 AND bh.data_class = 'storage_object'
 AND bh.subject_id = lm.subject_digest
WHERE bh.released_at_ms IS NULL;

CREATE TABLE storage_deletion_workflows_v1 (
  organization_id TEXT NOT NULL,
  subject_digest TEXT NOT NULL,
  correlation_id TEXT NOT NULL UNIQUE CHECK (length(correlation_id) = 36),
  inventory_digest TEXT NOT NULL CHECK (length(inventory_digest) = 64),
  stage TEXT NOT NULL CHECK (stage IN (
    'planned','tombstoned','origin_deleted','cache_purged','backup_deleted','verified','complete','restored'
  )),
  revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  guard_revision INTEGER NOT NULL CHECK (guard_revision BETWEEN 1 AND 9007199254740991),
  workflow_json TEXT NOT NULL CHECK (json_valid(workflow_json)),
  requested_at_ms INTEGER NOT NULL CHECK (requested_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, subject_digest),
  FOREIGN KEY (organization_id, subject_digest)
    REFERENCES storage_lifecycle_manifests_v1(organization_id, subject_digest) ON DELETE RESTRICT
);

CREATE TABLE storage_deletion_evidence_v1 (
  correlation_id TEXT NOT NULL REFERENCES storage_deletion_workflows_v1(correlation_id) ON DELETE RESTRICT,
  stage TEXT NOT NULL CHECK (stage IN (
    'tombstoned','origin_deleted','cache_purged','backup_deleted','verified'
  )),
  target_digest TEXT NOT NULL CHECK (length(target_digest) = 64),
  provider_receipt_digest TEXT NOT NULL CHECK (length(provider_receipt_digest) = 64),
  observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms BETWEEN 0 AND 9007199254740991),
  receipt_json TEXT NOT NULL CHECK (json_valid(receipt_json)),
  PRIMARY KEY (correlation_id, stage)
);
CREATE TRIGGER storage_deletion_evidence_v1_no_update
BEFORE UPDATE ON storage_deletion_evidence_v1
BEGIN SELECT RAISE(ABORT, 'storage_deletion_evidence_is_immutable'); END;
CREATE TRIGGER storage_deletion_evidence_v1_no_delete
BEFORE DELETE ON storage_deletion_evidence_v1
BEGIN SELECT RAISE(ABORT, 'storage_deletion_evidence_is_immutable'); END;

CREATE TABLE storage_cache_operations_v1 (
  plan_digest TEXT PRIMARY KEY NOT NULL CHECK (length(plan_digest) = 64),
  organization_id TEXT NOT NULL,
  object_key TEXT NOT NULL,
  from_generation INTEGER NOT NULL CHECK (from_generation BETWEEN 1 AND 9007199254740991),
  to_generation INTEGER NOT NULL CHECK (to_generation = from_generation + 1),
  deadline_ms INTEGER NOT NULL CHECK (deadline_ms BETWEEN 1 AND 9007199254740991),
  state TEXT NOT NULL CHECK (state IN ('pending','verified','failed')),
  plan_json TEXT NOT NULL CHECK (json_valid(plan_json)),
  receipt_json TEXT CHECK (receipt_json IS NULL OR json_valid(receipt_json)),
  positive_absent INTEGER CHECK (positive_absent IS NULL OR positive_absent IN (0,1)),
  negative_absent INTEGER CHECK (negative_absent IS NULL OR negative_absent IN (0,1)),
  verified_at_ms INTEGER CHECK (verified_at_ms IS NULL OR verified_at_ms BETWEEN 0 AND 9007199254740991),
  FOREIGN KEY (organization_id, object_key)
    REFERENCES storage_governed_objects_v1(organization_id, object_key) ON DELETE RESTRICT,
  CHECK ((state = 'verified') = (receipt_json IS NOT NULL AND positive_absent = 1
                                 AND negative_absent = 1 AND verified_at_ms IS NOT NULL))
);
CREATE INDEX storage_cache_operations_v1_pending_idx
  ON storage_cache_operations_v1(state, deadline_ms);

CREATE TABLE storage_quota_state_v1 (
  organization_id TEXT PRIMARY KEY NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  max_bytes INTEGER NOT NULL CHECK (max_bytes BETWEEN 1 AND 9007199254740991),
  max_objects INTEGER NOT NULL CHECK (max_objects BETWEEN 1 AND 9007199254740991),
  used_bytes INTEGER NOT NULL DEFAULT 0 CHECK (used_bytes BETWEEN 0 AND 9007199254740991),
  used_objects INTEGER NOT NULL DEFAULT 0 CHECK (used_objects BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision BETWEEN 1 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (used_bytes <= max_bytes AND used_objects <= max_objects)
);
INSERT INTO storage_quota_state_v1(
  organization_id, max_bytes, max_objects, used_bytes, used_objects, revision, updated_at_ms
)
SELECT o.id,
       MAX(5497558138880, COALESCE(SUM(g.bytes), 0)),
       MAX(10000000, COUNT(g.object_key)),
       COALESCE(SUM(g.bytes), 0),
       COUNT(g.object_key),
       1,
       o.updated_at_ms
FROM organizations o
LEFT JOIN storage_governed_objects_v1 g
  ON g.organization_id = o.id AND g.state <> 'erased'
GROUP BY o.id;
CREATE TRIGGER storage_quota_state_v1_organization_insert
AFTER INSERT ON organizations
BEGIN
  INSERT INTO storage_quota_state_v1(
    organization_id, max_bytes, max_objects, used_bytes, used_objects, revision, updated_at_ms
  ) VALUES (NEW.id, 5497558138880, 10000000, 0, 0, 1, NEW.updated_at_ms);
END;

CREATE TABLE storage_quota_reservations_v1 (
  reservation_id TEXT PRIMARY KEY NOT NULL CHECK (length(reservation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES storage_quota_state_v1(organization_id) ON DELETE RESTRICT,
  requested_bytes INTEGER NOT NULL CHECK (requested_bytes BETWEEN 1 AND 9007199254740991),
  state TEXT NOT NULL CHECK (state IN ('outstanding','committed','released','expired')),
  expected_quota_revision INTEGER NOT NULL CHECK (expected_quota_revision BETWEEN 1 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 1 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms),
  CHECK ((state = 'outstanding') = (completed_at_ms IS NULL))
);
CREATE INDEX storage_quota_reservations_v1_outstanding_idx
  ON storage_quota_reservations_v1(organization_id, state, expires_at_ms);

CREATE TABLE storage_governance_audit_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 9007199254740991),
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36),
  previous_digest TEXT NOT NULL CHECK (
    length(previous_digest) = 64 AND lower(previous_digest) = previous_digest
      AND previous_digest NOT GLOB '*[^0-9a-f]*'
  ),
  digest TEXT NOT NULL UNIQUE CHECK (
    length(digest) = 64 AND lower(digest) = digest
      AND digest NOT GLOB '*[^0-9a-f]*'
  ),
  record_json TEXT NOT NULL CHECK (
    json_valid(record_json)
      AND json_extract(record_json, '$.schema_version') = 1
      AND json_extract(record_json, '$.sequence') = sequence
      AND json_extract(record_json, '$.tenant_id') = organization_id
      AND json_extract(record_json, '$.correlation_id') = correlation_id
      AND json_extract(record_json, '$.previous_digest') = previous_digest
      AND json_extract(record_json, '$.digest') = digest
      AND json_extract(record_json, '$.occurred_at') = occurred_at_ms
  ),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, sequence)
);
CREATE TRIGGER storage_governance_audit_v1_chain_insert
BEFORE INSERT ON storage_governance_audit_v1
WHEN NOT EXISTS (
       SELECT 1 FROM storage_governance_audit_v1 existing
        WHERE existing.organization_id = NEW.organization_id
          AND existing.sequence = NEW.sequence AND existing.digest = NEW.digest
     )
 AND (
   NEW.sequence <> COALESCE((
     SELECT MAX(a.sequence) + 1 FROM storage_governance_audit_v1 a
      WHERE a.organization_id = NEW.organization_id
   ), 1)
   OR NEW.previous_digest <> COALESCE((
     SELECT a.digest FROM storage_governance_audit_v1 a
      WHERE a.organization_id = NEW.organization_id ORDER BY a.sequence DESC LIMIT 1
   ), '0000000000000000000000000000000000000000000000000000000000000000')
   OR NEW.occurred_at_ms < COALESCE((
     SELECT a.occurred_at_ms FROM storage_governance_audit_v1 a
      WHERE a.organization_id = NEW.organization_id ORDER BY a.sequence DESC LIMIT 1
   ), 0)
 )
BEGIN
  SELECT RAISE(IGNORE);
END;
CREATE TRIGGER storage_governance_audit_v1_no_update
BEFORE UPDATE ON storage_governance_audit_v1
BEGIN SELECT RAISE(ABORT, 'storage_governance_audit_is_immutable'); END;
CREATE TRIGGER storage_governance_audit_v1_no_delete
BEFORE DELETE ON storage_governance_audit_v1
BEGIN SELECT RAISE(ABORT, 'storage_governance_audit_is_immutable'); END;

-- Actual runtime reads may use a verified developer-domain bridge, but only an
-- exact active row from one of these two authoritative workflows is accepted.
CREATE VIEW storage_verified_domains_v1 AS
SELECT domain_ascii, organization_id, verification_version, 1 AS active
FROM storage_custom_domains_v1
WHERE state = 'active'
UNION
SELECT lower(d.domain_ascii), a.organization_id, 1 AS verification_version, 1 AS active
FROM developer_app_domains d
JOIN developer_apps a ON a.id = d.app_id
WHERE d.verified_at_ms IS NOT NULL
  AND a.organization_id IS NOT NULL
  AND a.status = 'active'
  AND a.deleted_at_ms IS NULL;
