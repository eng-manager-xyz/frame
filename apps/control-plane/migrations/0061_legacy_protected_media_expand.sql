PRAGMA foreign_keys = ON;

-- Scheduler and media-service credentials are provisioned by operators, not
-- by request handlers. Rotating or disabling a row invalidates every receipt
-- bound to that exact secret digest without placing the secret itself in D1.
CREATE TABLE legacy_protected_media_service_authorities_v1 (
  credential_subject_id TEXT PRIMARY KEY NOT NULL CHECK (
    credential_subject_id IN (
      'CRON_SECRET.v1','MEDIA_SERVER_WEBHOOK_SECRET.v1','FRAME_MEDIA_FLOW_TOKEN.v1'
    )
  ),
  credential_kind TEXT NOT NULL CHECK (
    credential_kind IN ('scheduler_secret','service_secret','flow_token')
  ),
  credential_key_version INTEGER NOT NULL CHECK (
    credential_key_version BETWEEN 1 AND 65535
  ),
  credential_digest TEXT NOT NULL CHECK (
    length(credential_digest) = 64 AND credential_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('active','disabled')),
  expires_at_ms INTEGER CHECK (
    expires_at_ms IS NULL OR expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  CHECK (
    (credential_subject_id = 'CRON_SECRET.v1' AND credential_kind = 'scheduler_secret')
    OR (credential_subject_id = 'MEDIA_SERVER_WEBHOOK_SECRET.v1'
      AND credential_kind = 'service_secret')
    OR (credential_subject_id = 'FRAME_MEDIA_FLOW_TOKEN.v1'
      AND credential_kind = 'flow_token')
  )
);

CREATE TABLE legacy_protected_media_ai_entitlements_v1 (
  user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  entitlement_revision INTEGER NOT NULL CHECK (
    entitlement_revision BETWEEN 0 AND 9007199254740991
  ),
  state TEXT NOT NULL CHECK (state IN ('active','disabled')),
  expires_at_ms INTEGER CHECK (
    expires_at_ms IS NULL OR expires_at_ms BETWEEN 1 AND 9007199254740991
  )
);

CREATE TABLE legacy_protected_media_receipts_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL CHECK (length(receipt_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (
    length(source_operation_id) = 23 AND source_operation_id LIKE 'cap-v1-%'
  ),
  operation_kind TEXT NOT NULL CHECK (
    operation_kind IN ('route','rpc','server_action','workflow')
  ),
  method TEXT NOT NULL CHECK (
    method IN ('GET','HEAD','POST','RPC','ACTION','WORKFLOW')
  ),
  surface_path TEXT NOT NULL CHECK (length(surface_path) BETWEEN 1 AND 512),
  auth_class TEXT NOT NULL CHECK (auth_class IN (
    'scheduler_secret','optional_session_or_share_capability','session',
    'public_or_flow_token','internal_service','public_edge_or_job_capability',
    'parent_derived'
  )),
  authority_class TEXT NOT NULL CHECK (authority_class IN (
    'scheduler_service','internal_service','flow_service','active_session','video_view',
    'video_owner','video_share','public_edge','job_status','organization_member',
    'video_view_ai_owner_entitled','video_owner_ai_entitled'
  )),
  principal_digest TEXT NOT NULL CHECK (
    length(principal_digest) = 64 AND principal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  actor_id TEXT CHECK (actor_id IS NULL OR length(actor_id) BETWEEN 1 AND 255),
  tenant_id TEXT CHECK (tenant_id IS NULL OR length(tenant_id) BETWEEN 1 AND 255),
  credential_kind TEXT NOT NULL CHECK (credential_kind IN (
    'session_token','scheduler_secret','service_secret','flow_token',
    'video_password_capability','space_password_capability',
    'public_video_capability','edge_read','job_capability','parent_capability'
  )),
  credential_subject_id TEXT NOT NULL CHECK (
    length(credential_subject_id) BETWEEN 1 AND 255
  ),
  credential_key_version INTEGER NOT NULL CHECK (
    credential_key_version BETWEEN 0 AND 9007199254740991
  ),
  credential_digest TEXT NOT NULL CHECK (
    length(credential_digest) = 64 AND credential_digest NOT GLOB '*[^0-9a-f]*'
  ),
  policy_proofs_json TEXT NOT NULL CHECK (
    json_valid(policy_proofs_json) AND json_type(policy_proofs_json)='array'
    AND json_array_length(policy_proofs_json) BETWEEN 0 AND 50
    AND length(policy_proofs_json)<=65536
  ),
  entitlement_kind TEXT CHECK (
    entitlement_kind IS NULL OR entitlement_kind='ai_owner'
  ),
  entitlement_subject_id TEXT CHECK (
    entitlement_subject_id IS NULL OR length(entitlement_subject_id)=36
  ),
  entitlement_revision INTEGER CHECK (
    entitlement_revision IS NULL OR entitlement_revision BETWEEN 0 AND 9007199254740991
  ),
  entitlement_expires_at_ms INTEGER CHECK (
    entitlement_expires_at_ms IS NULL
    OR entitlement_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  target_id TEXT CHECK (target_id IS NULL OR length(target_id) BETWEEN 1 AND 512),
  authority_binding_digest TEXT NOT NULL CHECK (
    length(authority_binding_digest) = 64
    AND authority_binding_digest NOT GLOB '*[^0-9a-f]*'
  ),
  parent_family TEXT CHECK (
    parent_family IS NULL OR parent_family IN ('protected_media','protected_integrations')
  ),
  parent_receipt_id TEXT CHECK (
    parent_receipt_id IS NULL OR length(parent_receipt_id)=36
  ),
  parent_request_digest TEXT CHECK (
    parent_request_digest IS NULL OR (
      length(parent_request_digest) = 64
      AND parent_request_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  parent_authority_binding_digest TEXT CHECK (
    parent_authority_binding_digest IS NULL OR (
      length(parent_authority_binding_digest)=64
      AND parent_authority_binding_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  execution_key_digest TEXT NOT NULL CHECK (
    length(execution_key_digest) = 64
    AND execution_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  replay_origin TEXT NOT NULL CHECK (
    replay_origin IN ('caller','natural','grant','workflow')
  ),
  idempotency_mode TEXT NOT NULL CHECK (idempotency_mode IN ('required','forbidden')),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  payload_digest TEXT NOT NULL CHECK (
    length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_descriptor_json TEXT NOT NULL CHECK (
    json_valid(request_descriptor_json)
    AND json_type(request_descriptor_json) = 'object'
    AND json_extract(request_descriptor_json, '$.schema_version')
      = 'frame.legacy-protected-media-request.v2'
    AND length(request_descriptor_json) <= 262144
  ),
  sealed_request_ref TEXT CHECK (
    sealed_request_ref IS NULL OR (
      length(sealed_request_ref) = 84
      AND substr(sealed_request_ref,1,20) = 'frame-pm-request-v1:'
      AND substr(sealed_request_ref,21) NOT GLOB '*[^0-9a-f]*'
    )
  ),
  sealed_request_digest TEXT CHECK (
    sealed_request_digest IS NULL OR (
      length(sealed_request_digest) = 64
      AND sealed_request_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  terminal_kind TEXT NOT NULL CHECK (
    terminal_kind IN ('json','redirect','binary','event_stream')
  ),
  executor_kind TEXT NOT NULL CHECK (
    executor_kind IN ('gstreamer','provider','control_plane')
  ),
  provider_required INTEGER NOT NULL CHECK (provider_required IN (0,1)),
  state TEXT NOT NULL DEFAULT 'pending_execution_evidence' CHECK (
    state IN ('pending_execution_evidence','verified','dead_letter')
  ),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  UNIQUE (source_operation_id, principal_digest, execution_key_digest),
  CHECK (
    (sealed_request_ref IS NULL AND sealed_request_digest IS NULL)
    OR (sealed_request_ref IS NOT NULL AND sealed_request_digest = payload_digest)
  ),
  CHECK (
    (entitlement_kind IS NULL AND entitlement_subject_id IS NULL
      AND entitlement_revision IS NULL AND entitlement_expires_at_ms IS NULL)
    OR (entitlement_kind='ai_owner' AND entitlement_subject_id IS NOT NULL
      AND entitlement_revision IS NOT NULL)
  ),
  CHECK (
    (operation_kind = 'workflow' AND replay_origin = 'workflow'
      AND parent_family IS NOT NULL AND parent_receipt_id IS NOT NULL
      AND parent_request_digest IS NOT NULL
      AND parent_authority_binding_digest IS NOT NULL)
    OR (operation_kind <> 'workflow'
      AND parent_family IS NULL AND parent_receipt_id IS NULL
      AND parent_request_digest IS NULL
      AND parent_authority_binding_digest IS NULL)
  ),
  CHECK (
    (state = 'pending_execution_evidence' AND completed_at_ms IS NULL)
    OR (state IN ('verified','dead_letter') AND completed_at_ms IS NOT NULL)
  ),
  CHECK (
    (auth_class = 'scheduler_secret' AND actor_id IS NULL
      AND credential_kind = 'scheduler_secret'
      AND credential_subject_id = 'CRON_SECRET.v1'
      AND credential_key_version BETWEEN 1 AND 65535
      AND authority_class = 'scheduler_service')
    OR (auth_class = 'internal_service' AND actor_id IS NULL
      AND credential_kind = 'service_secret'
      AND credential_subject_id = 'MEDIA_SERVER_WEBHOOK_SECRET.v1'
      AND credential_key_version BETWEEN 1 AND 65535
      AND authority_class = 'internal_service')
    OR (auth_class = 'session' AND actor_id IS NOT NULL
      AND credential_kind = 'session_token'
      AND length(credential_subject_id) = 36
      AND credential_key_version BETWEEN 1 AND 65535
      AND authority_class IN (
        'active_session','video_view','video_owner','organization_member',
        'video_view_ai_owner_entitled','video_owner_ai_entitled'
      ))
    OR (auth_class = 'optional_session_or_share_capability' AND (
      (actor_id IS NOT NULL AND credential_kind = 'session_token'
        AND length(credential_subject_id) = 36
        AND credential_key_version BETWEEN 1 AND 65535
        AND authority_class = 'video_view')
      OR (actor_id IS NULL AND credential_kind IN (
          'video_password_capability','space_password_capability',
          'public_video_capability'
        ) AND length(credential_subject_id) = 36
        AND authority_class = 'video_share')
    ))
    OR (auth_class = 'public_or_flow_token' AND actor_id IS NULL
      AND credential_kind = 'flow_token'
      AND credential_subject_id = 'FRAME_MEDIA_FLOW_TOKEN.v1'
      AND credential_key_version BETWEEN 1 AND 65535
      AND authority_class = 'flow_service')
    OR (auth_class = 'public_edge_or_job_capability' AND actor_id IS NULL AND (
      (credential_kind = 'edge_read' AND credential_key_version = 1
        AND credential_subject_id = source_operation_id
        AND authority_class = 'public_edge' AND target_id IS NULL)
      OR (credential_kind = 'job_capability'
        AND credential_subject_id = target_id AND authority_class = 'job_status')
    ))
    OR (auth_class='parent_derived' AND operation_kind='workflow')
  )
);
CREATE INDEX legacy_protected_media_receipts_target_v1
  ON legacy_protected_media_receipts_v1(target_id,created_at_ms,receipt_id);
CREATE INDEX legacy_protected_media_receipts_parent_v1
  ON legacy_protected_media_receipts_v1(parent_family,parent_receipt_id,receipt_id);

-- Cross-family workflow parents use one neutral, immutable registry. 0062
-- mirrors protected-integration receipts into the same table. This avoids a
-- migration-order FK cycle while retaining exact request and authority data.
CREATE TABLE legacy_protected_effect_parent_registry_v1 (
  parent_family TEXT NOT NULL CHECK (
    parent_family IN ('protected_media','protected_integrations')
  ),
  parent_receipt_id TEXT NOT NULL CHECK (length(parent_receipt_id)=36),
  source_operation_id TEXT NOT NULL CHECK (
    length(source_operation_id)=23 AND source_operation_id LIKE 'cap-v1-%'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest)=64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  actor_id TEXT,
  tenant_id TEXT,
  target_id TEXT,
  auth_class TEXT NOT NULL CHECK (length(auth_class) BETWEEN 1 AND 96),
  authority_class TEXT NOT NULL CHECK (length(authority_class) BETWEEN 1 AND 96),
  credential_kind TEXT NOT NULL CHECK (length(credential_kind) BETWEEN 1 AND 96),
  credential_subject_id TEXT CHECK (
    credential_subject_id IS NULL OR length(credential_subject_id) BETWEEN 1 AND 255
  ),
  credential_key_version INTEGER CHECK (
    credential_key_version IS NULL
    OR credential_key_version BETWEEN 0 AND 9007199254740991
  ),
  credential_digest TEXT CHECK (
    credential_digest IS NULL OR (
      length(credential_digest)=64 AND credential_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  credential_expires_at_ms INTEGER CHECK (
    credential_expires_at_ms IS NULL
    OR credential_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  policy_proofs_json TEXT NOT NULL CHECK (
    json_valid(policy_proofs_json) AND json_type(policy_proofs_json)='array'
    AND json_array_length(policy_proofs_json) BETWEEN 0 AND 50
    AND length(policy_proofs_json)<=65536
  ),
  entitlement_kind TEXT,
  entitlement_subject_id TEXT,
  entitlement_revision INTEGER,
  entitlement_expires_at_ms INTEGER,
  authority_binding_digest TEXT NOT NULL CHECK (
    length(authority_binding_digest)=64
    AND authority_binding_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN (
    'pending_execution_evidence','pending_provider_evidence','verified','dead_letter'
  )),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY(parent_family,parent_receipt_id),
  CHECK (
    (credential_kind='none' AND credential_subject_id IS NULL
      AND credential_key_version IS NULL AND credential_digest IS NULL
      AND credential_expires_at_ms IS NULL)
    OR (credential_kind<>'none' AND credential_subject_id IS NOT NULL
      AND credential_digest IS NOT NULL
      AND (credential_key_version IS NOT NULL OR credential_kind='api_key'))
  ),
  CHECK (
    (state IN ('pending_execution_evidence','pending_provider_evidence')
      AND completed_at_ms IS NULL)
    OR (state IN ('verified','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_protected_effect_parent_registry_operation_v1
  ON legacy_protected_effect_parent_registry_v1(
    parent_family,source_operation_id,state,parent_receipt_id
  );

CREATE TABLE legacy_protected_effect_parent_edges_v1 (
  parent_family TEXT NOT NULL CHECK (
    parent_family IN ('protected_media','protected_integrations')
  ),
  parent_operation_id TEXT NOT NULL CHECK (
    length(parent_operation_id)=23 AND parent_operation_id LIKE 'cap-v1-%'
  ),
  child_family TEXT NOT NULL CHECK (
    child_family IN ('protected_media','protected_integrations')
  ),
  child_operation_id TEXT NOT NULL CHECK (
    length(child_operation_id)=23 AND child_operation_id LIKE 'cap-v1-%'
  ),
  target_binding_rule TEXT NOT NULL DEFAULT 'same' CHECK (
    target_binding_rule IN ('same','child_derived')
  ),
  PRIMARY KEY(parent_family,parent_operation_id,child_family,child_operation_id)
);
INSERT INTO legacy_protected_effect_parent_edges_v1 (
  parent_family,parent_operation_id,child_family,child_operation_id
) VALUES
  ('protected_media','cap-v1-c1ae43fcf8ad7018','protected_media','cap-v1-3e0dec6125f270bf'),
  ('protected_media','cap-v1-39909646286251af','protected_media','cap-v1-3e0dec6125f270bf'),
  ('protected_media','cap-v1-44259057076456cf','protected_media','cap-v1-b3fac7b3df933825'),
  ('protected_media','cap-v1-b3fac7b3df933825','protected_media','cap-v1-4cff2b6f3cd102f5'),
  ('protected_media','cap-v1-4cff2b6f3cd102f5','protected_media','cap-v1-39c33826cf514552'),
  ('protected_media','cap-v1-243668046d7d1c3a','protected_media','cap-v1-39c33826cf514552'),
  ('protected_media','cap-v1-243668046d7d1c3a','protected_media','cap-v1-7868ad041c2754df'),
  ('protected_media','cap-v1-39c33826cf514552','protected_media','cap-v1-7868ad041c2754df'),
  ('protected_media','cap-v1-94a9944ce37fa085','protected_media','cap-v1-6d73e4dfdca61f06'),
  ('protected_media','cap-v1-187fbaf66d21b311','protected_media','cap-v1-6d73e4dfdca61f06'),
  ('protected_media','cap-v1-4b12db3b619dce8f','protected_media','cap-v1-6d73e4dfdca61f06'),
  ('protected_media','cap-v1-6d73e4dfdca61f06','protected_media','cap-v1-0d39ec834208980f'),
  ('protected_media','cap-v1-a8dc17023685b8c0','protected_media','cap-v1-59ce5faf2189c1a1'),
  ('protected_media','cap-v1-8e495fce95e6282b','protected_media','cap-v1-59ce5faf2189c1a1'),
  ('protected_media','cap-v1-0d39ec834208980f','protected_media','cap-v1-43c049b69abb6704'),
  ('protected_media','cap-v1-7868ad041c2754df','protected_media','cap-v1-43c049b69abb6704'),
  ('protected_media','cap-v1-59ce5faf2189c1a1','protected_media','cap-v1-43c049b69abb6704'),
  ('protected_media','cap-v1-43c049b69abb6704','protected_media','cap-v1-3e0dec6125f270bf'),
  ('protected_media','cap-v1-3e0dec6125f270bf','protected_media','cap-v1-c79bf3eeab46cbf0'),
  ('protected_integrations','cap-v1-d9b654b30f6c362a','protected_media','cap-v1-39c33826cf514552'),
  ('protected_integrations','cap-v1-d9b654b30f6c362a','protected_media','cap-v1-3e0dec6125f270bf'),
  ('protected_integrations','cap-v1-d9b654b30f6c362a','protected_media','cap-v1-43c049b69abb6704'),
  ('protected_media','cap-v1-94a9944ce37fa085','protected_integrations','cap-v1-b9fcb0fbd25b2234');

UPDATE legacy_protected_effect_parent_edges_v1 SET target_binding_rule='child_derived'
WHERE (parent_family='protected_media'
    AND parent_operation_id='cap-v1-b3fac7b3df933825'
    AND child_family='protected_media'
    AND child_operation_id='cap-v1-4cff2b6f3cd102f5')
   OR (parent_family='protected_integrations'
    AND parent_operation_id='cap-v1-d9b654b30f6c362a'
    AND child_family='protected_media')
   OR (parent_family='protected_media'
    AND parent_operation_id='cap-v1-94a9944ce37fa085'
    AND child_family='protected_integrations'
    AND child_operation_id='cap-v1-b9fcb0fbd25b2234');

CREATE TRIGGER legacy_protected_effect_parent_edges_immutable_v1
BEFORE UPDATE ON legacy_protected_effect_parent_edges_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_effect_parent_edge_immutable_v1'); END;
CREATE TRIGGER legacy_protected_effect_parent_edges_no_delete_v1
BEFORE DELETE ON legacy_protected_effect_parent_edges_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_effect_parent_edge_immutable_v1'); END;
CREATE TRIGGER legacy_protected_effect_parent_registry_no_delete_v1
BEFORE DELETE ON legacy_protected_effect_parent_registry_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_effect_parent_registry_immutable_v1'); END;
CREATE TRIGGER legacy_protected_effect_parent_registry_update_gate_v1
BEFORE UPDATE ON legacy_protected_effect_parent_registry_v1
WHEN NOT (
  OLD.parent_family=NEW.parent_family
  AND OLD.parent_receipt_id=NEW.parent_receipt_id
  AND OLD.source_operation_id=NEW.source_operation_id
  AND OLD.request_digest=NEW.request_digest
  AND OLD.actor_id IS NEW.actor_id AND OLD.tenant_id IS NEW.tenant_id
  AND OLD.target_id IS NEW.target_id AND OLD.auth_class=NEW.auth_class
  AND OLD.authority_class=NEW.authority_class
  AND OLD.credential_kind=NEW.credential_kind
  AND OLD.credential_subject_id IS NEW.credential_subject_id
  AND OLD.credential_key_version IS NEW.credential_key_version
  AND OLD.credential_digest IS NEW.credential_digest
  AND OLD.credential_expires_at_ms IS NEW.credential_expires_at_ms
  AND OLD.policy_proofs_json=NEW.policy_proofs_json
  AND OLD.entitlement_kind IS NEW.entitlement_kind
  AND OLD.entitlement_subject_id IS NEW.entitlement_subject_id
  AND OLD.entitlement_revision IS NEW.entitlement_revision
  AND OLD.entitlement_expires_at_ms IS NEW.entitlement_expires_at_ms
  AND OLD.authority_binding_digest=NEW.authority_binding_digest
  AND OLD.created_at_ms=NEW.created_at_ms
  AND OLD.state IN ('pending_execution_evidence','pending_provider_evidence')
  AND NEW.state IN ('verified','dead_letter')
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_effect_parent_registry_immutable_v1'); END;


-- Exact credential liveness is evaluated independently from video policy.
-- This prevents a password proof from replacing a session credential (and
-- vice versa), while still permitting the composite policy Cap requires.
CREATE VIEW legacy_protected_media_live_credentials_v1 AS
SELECT receipt.receipt_id,
  CASE
    WHEN receipt.credential_kind='session_token' THEN (
      SELECT MIN(session.idle_expires_at_ms,session.absolute_expires_at_ms)
      FROM auth_sessions_v2 session
      WHERE session.id=receipt.credential_subject_id
    )
    WHEN receipt.credential_kind IN ('scheduler_secret','service_secret','flow_token') THEN (
      SELECT COALESCE(authority.expires_at_ms,9007199254740991)
      FROM legacy_protected_media_service_authorities_v1 authority
      WHERE authority.credential_subject_id=receipt.credential_subject_id
    )
    WHEN receipt.credential_kind='parent_capability' THEN (
      SELECT COALESCE(origin.credential_expires_at_ms,9007199254740991)
      FROM legacy_protected_effect_parent_registry_v1 origin
      WHERE origin.parent_family || ':' || origin.parent_receipt_id
        =receipt.credential_subject_id
    )
    ELSE 9007199254740991
  END AS credential_expires_at_ms
FROM legacy_protected_media_receipts_v1 receipt
WHERE
  (receipt.credential_kind='edge_read'
    AND receipt.credential_subject_id=receipt.source_operation_id
    AND receipt.credential_key_version=1)
  OR (receipt.credential_kind IN ('scheduler_secret','service_secret','flow_token')
    AND EXISTS (
      SELECT 1 FROM legacy_protected_media_service_authorities_v1 authority
      WHERE authority.credential_subject_id=receipt.credential_subject_id
        AND authority.credential_kind=receipt.credential_kind
        AND authority.credential_key_version=receipt.credential_key_version
        AND authority.credential_digest=receipt.credential_digest
        AND authority.state='active'
    ))
  OR (receipt.credential_kind='session_token' AND EXISTS (
    SELECT 1 FROM auth_sessions_v2 session
    JOIN auth_identities_v2 identity ON identity.user_id=session.user_id
    JOIN users actor ON actor.id=session.user_id
    WHERE session.id=receipt.credential_subject_id
      AND session.user_id=receipt.actor_id
      AND session.token_key_version=receipt.credential_key_version
      AND session.token_digest=receipt.credential_digest
      AND session.state='active' AND session.revoked_at_ms IS NULL
      AND session.session_version=identity.session_version
      AND actor.status='active' AND actor.deleted_at_ms IS NULL
      AND (receipt.tenant_id IS NULL OR EXISTS (
        SELECT 1 FROM organization_members tenant_member
        JOIN organizations tenant ON tenant.id=tenant_member.organization_id
        WHERE tenant_member.organization_id=receipt.tenant_id
          AND tenant_member.user_id=actor.id AND tenant_member.state='active'
          AND tenant.status='active'
      ))
  ))
  OR (receipt.credential_kind='job_capability' AND EXISTS (
    SELECT 1 FROM media_jobs job JOIN videos video ON video.id=job.video_id
    WHERE job.id=receipt.target_id AND job.id=receipt.credential_subject_id
      AND job.updated_at_ms=receipt.credential_key_version
      AND video.deleted_at_ms IS NULL
  ))
  OR (receipt.credential_kind='parent_capability' AND EXISTS (
    SELECT 1 FROM legacy_protected_effect_parent_registry_v1 origin
    WHERE origin.parent_family || ':' || origin.parent_receipt_id
        =receipt.credential_subject_id
      AND origin.created_at_ms=receipt.credential_key_version
      AND origin.authority_binding_digest=receipt.credential_digest
      AND origin.state<>'dead_letter'
  ))
  OR (receipt.credential_kind='video_password_capability' AND EXISTS (
    SELECT 1 FROM json_each(receipt.policy_proofs_json) proof
    JOIN legacy_collaboration_video_aliases_v1 alias
      ON alias.legacy_video_id=json_extract(proof.value,'$.target_id')
    JOIN videos video ON video.id=alias.mapped_video_id
    WHERE json_extract(proof.value,'$.kind')='video_password'
      AND json_extract(proof.value,'$.subject_id')=receipt.credential_subject_id
      AND CAST(json_extract(proof.value,'$.revision') AS INTEGER)
        =receipt.credential_key_version
      AND video.id=receipt.credential_subject_id
      AND video.legacy_property_revision=receipt.credential_key_version
      AND video.legacy_password_hash IS NOT NULL
      AND video.deleted_at_ms IS NULL
  ))
  OR (receipt.credential_kind='space_password_capability' AND EXISTS (
    SELECT 1 FROM json_each(receipt.policy_proofs_json) proof
    JOIN legacy_collaboration_video_aliases_v1 alias
      ON alias.legacy_video_id=json_extract(proof.value,'$.target_id')
    JOIN space_videos placement ON placement.video_id=alias.mapped_video_id
    JOIN spaces space ON space.id=placement.space_id
    WHERE json_extract(proof.value,'$.kind')='space_password'
      AND json_extract(proof.value,'$.subject_id')=receipt.credential_subject_id
      AND CAST(json_extract(proof.value,'$.revision') AS INTEGER)
        =receipt.credential_key_version
      AND space.id=receipt.credential_subject_id
      AND space.legacy_password_revision=receipt.credential_key_version
      AND space.legacy_password_hash IS NOT NULL
      AND space.deleted_at_ms IS NULL
  ))
  OR (receipt.credential_kind='public_video_capability' AND EXISTS (
    SELECT 1 FROM json_each(receipt.policy_proofs_json) proof
    JOIN legacy_collaboration_video_aliases_v1 alias
      ON alias.legacy_video_id=json_extract(proof.value,'$.target_id')
    JOIN videos video ON video.id=alias.mapped_video_id
    WHERE json_extract(proof.value,'$.kind')='unprotected_video_policy'
      AND video.id=receipt.credential_subject_id
      AND video.legacy_property_revision=receipt.credential_key_version
      AND video.legacy_public=1 AND video.legacy_password_hash IS NULL
      AND video.deleted_at_ms IS NULL
      AND NOT EXISTS (
        SELECT 1 FROM space_videos placement
        JOIN spaces space ON space.id=placement.space_id
        WHERE placement.video_id=video.id AND space.deleted_at_ms IS NULL
          AND space.legacy_password_hash IS NOT NULL
      )
  ));

-- Every JSON proof must still resolve to the same video/space revision and to
-- the same owner/member/public-email decision. Password hashes remain outside
-- receipts; their monotonic revision and non-null state are the live fence.
CREATE VIEW legacy_protected_media_live_video_policy_v1 AS
SELECT receipt.receipt_id
FROM legacy_protected_media_receipts_v1 receipt
WHERE receipt.authority_class IN (
    'video_view','video_owner','video_share',
    'video_view_ai_owner_entitled','video_owner_ai_entitled'
  )
  AND json_array_length(receipt.policy_proofs_json)>0
  AND (
    receipt.target_id IS NULL
    OR (
      json_array_length(receipt.policy_proofs_json)=1
      AND json_extract(receipt.policy_proofs_json,'$[0].target_id')=receipt.target_id
    )
  )
  AND NOT EXISTS (
    SELECT 1 FROM json_each(receipt.policy_proofs_json) proof
    WHERE json_type(proof.value,'$.target_id')<>'text'
      OR json_type(proof.value,'$.kind')<>'text'
      OR json_type(proof.value,'$.subject_id')<>'text'
      OR json_type(proof.value,'$.revision')<>'integer'
      OR json_type(proof.value,'$.audit_digest')<>'text'
      OR length(json_extract(proof.value,'$.target_id')) NOT BETWEEN 1 AND 255
      OR length(json_extract(proof.value,'$.subject_id'))<>36
      OR CAST(json_extract(proof.value,'$.revision') AS INTEGER)
        NOT BETWEEN 0 AND 9007199254740991
      OR length(json_extract(proof.value,'$.audit_digest'))<>64
      OR json_extract(proof.value,'$.audit_digest') GLOB '*[^0-9a-f]*'
      OR json_extract(proof.value,'$.kind') NOT IN (
        'owner_bypass','video_password','space_password','unprotected_video_policy'
      )
      OR NOT EXISTS (
        SELECT 1
        FROM legacy_collaboration_video_aliases_v1 alias
        JOIN videos video ON video.id=alias.mapped_video_id
        LEFT JOIN organizations organization ON organization.id=video.organization_id
        LEFT JOIN users actor ON actor.id=receipt.actor_id
        WHERE alias.legacy_video_id=json_extract(proof.value,'$.target_id')
          AND video.deleted_at_ms IS NULL
          AND (
            (json_extract(proof.value,'$.kind')='owner_bypass'
              AND receipt.actor_id=video.owner_id
              AND json_extract(proof.value,'$.subject_id')=video.id
              AND CAST(json_extract(proof.value,'$.revision') AS INTEGER)
                =video.legacy_property_revision)
            OR (
              json_extract(proof.value,'$.kind')<>'owner_bypass'
              AND (
                EXISTS (
                  SELECT 1 FROM organization_members member
                  WHERE member.organization_id=video.organization_id
                    AND member.user_id=receipt.actor_id AND member.state='active'
                )
                OR EXISTS (
                  SELECT 1 FROM space_videos membership_placement
                  JOIN space_members member
                    ON member.space_id=membership_placement.space_id
                  JOIN spaces membership_space
                    ON membership_space.id=membership_placement.space_id
                  WHERE membership_placement.video_id=video.id
                    AND member.user_id=receipt.actor_id
                    AND membership_space.deleted_at_ms IS NULL
                )
                OR (
                  video.legacy_public=1
                  AND (
                    COALESCE(TRIM(organization.legacy_allowed_email_restriction),'')=''
                    OR (actor.email IS NOT NULL AND EXISTS (
                      WITH RECURSIVE restriction_parts(rest,entry) AS (
                        SELECT COALESCE(organization.legacy_allowed_email_restriction,'') || ',', ''
                        UNION ALL
                        SELECT substr(rest,instr(rest,',')+1),
                          trim(substr(rest,1,instr(rest,',')-1))
                        FROM restriction_parts WHERE instr(rest,',')>0
                      )
                      SELECT 1 FROM restriction_parts
                      WHERE entry<>'' AND (
                        (instr(entry,'@')>0 AND lower(entry)=lower(actor.email))
                        OR (instr(entry,'@')=0
                          AND length(actor.email)>length(entry)+1
                          AND substr(lower(actor.email),-length(entry)-1)
                            ='@' || lower(entry))
                      )
                    ))
                  )
                )
              )
              AND (
                (json_extract(proof.value,'$.kind')='video_password'
                  AND json_extract(proof.value,'$.subject_id')=video.id
                  AND CAST(json_extract(proof.value,'$.revision') AS INTEGER)
                    =video.legacy_property_revision
                  AND video.legacy_password_hash IS NOT NULL)
                OR (json_extract(proof.value,'$.kind')='space_password'
                  AND EXISTS (
                    SELECT 1 FROM space_videos placement
                    JOIN spaces space ON space.id=placement.space_id
                    WHERE placement.video_id=video.id
                      AND space.id=json_extract(proof.value,'$.subject_id')
                      AND space.legacy_password_revision
                        =CAST(json_extract(proof.value,'$.revision') AS INTEGER)
                      AND space.legacy_password_hash IS NOT NULL
                      AND space.deleted_at_ms IS NULL
                  ))
                OR (json_extract(proof.value,'$.kind')='unprotected_video_policy'
                  AND json_extract(proof.value,'$.subject_id')=video.id
                  AND CAST(json_extract(proof.value,'$.revision') AS INTEGER)
                    =video.legacy_property_revision
                  AND video.legacy_password_hash IS NULL
                  AND NOT EXISTS (
                    SELECT 1 FROM space_videos placement
                    JOIN spaces space ON space.id=placement.space_id
                    WHERE placement.video_id=video.id
                      AND space.deleted_at_ms IS NULL
                      AND space.legacy_password_hash IS NOT NULL
                  ))
              )
            )
          )
      )
  )
  AND (
    receipt.authority_class NOT IN ('video_owner','video_owner_ai_entitled')
    OR NOT EXISTS (
      SELECT 1 FROM json_each(receipt.policy_proofs_json) proof
      WHERE json_extract(proof.value,'$.kind')<>'owner_bypass'
    )
  );

-- Replay, child launch, execution evidence, and terminal delivery all consume
-- this same projection. Any exact credential, policy, entitlement, membership,
-- password revision, placement, or target drift removes the row immediately.
CREATE VIEW legacy_protected_media_live_authority_v1 AS
SELECT receipt.receipt_id,
  CASE
    WHEN receipt.entitlement_expires_at_ms IS NOT NULL
      AND receipt.entitlement_expires_at_ms<credential.credential_expires_at_ms
      THEN receipt.entitlement_expires_at_ms
    ELSE credential.credential_expires_at_ms
  END AS authority_expires_at_ms
FROM legacy_protected_media_receipts_v1 receipt
JOIN legacy_protected_media_live_credentials_v1 credential
  ON credential.receipt_id=receipt.receipt_id
WHERE
  (receipt.authority_class='public_edge'
    AND receipt.credential_kind='edge_read' AND receipt.target_id IS NULL)
  OR (receipt.authority_class='scheduler_service'
    AND receipt.credential_kind='scheduler_secret')
  OR (receipt.authority_class='internal_service'
    AND receipt.credential_kind='service_secret')
  OR (receipt.authority_class='flow_service'
    AND receipt.credential_kind='flow_token')
  OR (receipt.authority_class='active_session'
    AND receipt.credential_kind='session_token'
    AND json_array_length(receipt.policy_proofs_json)=0
    AND receipt.entitlement_kind IS NULL)
  OR (receipt.authority_class='job_status'
    AND receipt.credential_kind='job_capability')
  OR (receipt.authority_class='organization_member'
    AND receipt.credential_kind='session_token'
    AND json_array_length(receipt.policy_proofs_json)=0
    AND EXISTS (
      SELECT 1 FROM legacy_user_account_organization_ids_v1 alias
      JOIN organization_members member ON member.organization_id=alias.organization_id
      JOIN organizations organization ON organization.id=member.organization_id
      WHERE alias.legacy_organization_id=receipt.target_id
        AND member.user_id=receipt.actor_id AND member.state='active'
        AND organization.status='active'
    ))
  OR (receipt.authority_class IN ('video_view','video_owner')
    AND receipt.credential_kind='session_token'
    AND receipt.entitlement_kind IS NULL
    AND EXISTS (
      SELECT 1 FROM legacy_protected_media_live_video_policy_v1 policy
      WHERE policy.receipt_id=receipt.receipt_id
    ))
  OR (receipt.authority_class='video_share'
    AND receipt.credential_kind IN (
      'video_password_capability','space_password_capability',
      'public_video_capability','parent_capability'
    )
    AND receipt.entitlement_kind IS NULL
    AND EXISTS (
      SELECT 1 FROM legacy_protected_media_live_video_policy_v1 policy
      WHERE policy.receipt_id=receipt.receipt_id
    ))
  OR (receipt.authority_class IN (
      'video_view_ai_owner_entitled','video_owner_ai_entitled'
    )
    AND receipt.credential_kind='session_token'
    AND receipt.entitlement_kind='ai_owner'
    AND EXISTS (
      SELECT 1 FROM legacy_protected_media_live_video_policy_v1 policy
      WHERE policy.receipt_id=receipt.receipt_id
    )
    AND EXISTS (
      SELECT 1 FROM legacy_protected_media_ai_entitlements_v1 entitlement
      WHERE entitlement.user_id=receipt.entitlement_subject_id
        AND entitlement.entitlement_revision=receipt.entitlement_revision
        AND entitlement.expires_at_ms IS receipt.entitlement_expires_at_ms
        AND entitlement.state='active'
    )
    AND NOT EXISTS (
      SELECT 1 FROM json_each(receipt.policy_proofs_json) proof
      WHERE NOT EXISTS (
        SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
        JOIN videos video ON video.id=alias.mapped_video_id
        WHERE alias.legacy_video_id=json_extract(proof.value,'$.target_id')
          AND video.owner_id=receipt.entitlement_subject_id
          AND video.deleted_at_ms IS NULL
      )
    ));

CREATE TRIGGER legacy_protected_media_receipt_authority_gate_v1
AFTER INSERT ON legacy_protected_media_receipts_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_media_live_authority_v1 live
  WHERE live.receipt_id=NEW.receipt_id
    AND live.authority_expires_at_ms>NEW.created_at_ms
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_authority_stale_v1'); END;

CREATE TRIGGER legacy_protected_media_workflow_parent_gate_v1
BEFORE INSERT ON legacy_protected_media_receipts_v1
WHEN NEW.operation_kind = 'workflow' AND NOT EXISTS (
  SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
  JOIN legacy_protected_effect_parent_edges_v1 edge
    ON edge.parent_family = parent.parent_family
   AND edge.parent_operation_id = parent.source_operation_id
   AND edge.child_family = 'protected_media'
   AND edge.child_operation_id = NEW.source_operation_id
  WHERE parent.parent_family = NEW.parent_family
    AND parent.parent_receipt_id = NEW.parent_receipt_id
    AND parent.request_digest = NEW.parent_request_digest
    AND parent.state <> 'dead_letter'
    AND parent.created_at_ms <= NEW.created_at_ms
    AND parent.actor_id IS NEW.actor_id
    AND parent.tenant_id IS NEW.tenant_id
    AND (
      (parent.credential_kind=NEW.credential_kind
        AND parent.credential_subject_id IS NEW.credential_subject_id
        AND parent.credential_key_version IS NEW.credential_key_version
        AND parent.credential_digest IS NEW.credential_digest)
      OR (parent.credential_kind='none'
        AND NEW.credential_kind='parent_capability'
        AND NEW.credential_subject_id
          =parent.parent_family || ':' || parent.parent_receipt_id
        AND NEW.credential_key_version=parent.created_at_ms
        AND NEW.credential_digest=parent.authority_binding_digest)
    )
    AND (
      parent.policy_proofs_json=NEW.policy_proofs_json
      OR (parent.source_operation_id='cap-v1-d9b654b30f6c362a'
        AND json_array_length(parent.policy_proofs_json)
          =json_array_length(NEW.policy_proofs_json)
        AND NOT EXISTS (
          SELECT 1 FROM json_each(parent.policy_proofs_json) parent_proof
          LEFT JOIN json_each(NEW.policy_proofs_json) child_proof
            ON child_proof.key=parent_proof.key
          WHERE json_extract(parent_proof.value,'$.kind')
              IS NOT json_extract(child_proof.value,'$.kind')
            OR json_extract(parent_proof.value,'$.subject_id')
              IS NOT json_extract(child_proof.value,'$.subject_id')
            OR json_extract(parent_proof.value,'$.revision')
              IS NOT json_extract(child_proof.value,'$.revision')
            OR json_extract(parent_proof.value,'$.audit_digest')
              IS NOT json_extract(child_proof.value,'$.audit_digest')
            OR NOT EXISTS (
              SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
              WHERE alias.mapped_video_id
                    =json_extract(parent_proof.value,'$.target_id')
                AND alias.legacy_video_id
                    =json_extract(child_proof.value,'$.target_id')
            )
        ))
    )
    AND parent.entitlement_kind IS NEW.entitlement_kind
    AND parent.entitlement_subject_id IS NEW.entitlement_subject_id
    AND parent.entitlement_revision IS NEW.entitlement_revision
    AND parent.entitlement_expires_at_ms IS NEW.entitlement_expires_at_ms
    AND (
      (edge.target_binding_rule='same' AND parent.target_id IS NEW.target_id)
      OR (edge.target_binding_rule='child_derived' AND NEW.target_id IS NOT NULL
        AND (
          parent.source_operation_id<>'cap-v1-d9b654b30f6c362a'
          OR EXISTS (
            SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
            WHERE alias.mapped_video_id=parent.target_id
              AND alias.legacy_video_id=NEW.target_id
          )
        ))
    )
    AND parent.authority_binding_digest = NEW.parent_authority_binding_digest
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_workflow_parent_invalid_v1'); END;

CREATE TRIGGER legacy_protected_media_parent_registry_insert_v1
AFTER INSERT ON legacy_protected_media_receipts_v1
BEGIN
  INSERT INTO legacy_protected_effect_parent_registry_v1 (
    parent_family,parent_receipt_id,source_operation_id,request_digest,
    actor_id,tenant_id,target_id,auth_class,authority_class,
    credential_kind,credential_subject_id,credential_key_version,credential_digest,
    credential_expires_at_ms,policy_proofs_json,entitlement_kind,entitlement_subject_id,
    entitlement_revision,entitlement_expires_at_ms,authority_binding_digest,
    state,created_at_ms,completed_at_ms
  ) VALUES (
    'protected_media',NEW.receipt_id,NEW.source_operation_id,NEW.request_digest,
    NEW.actor_id,NEW.tenant_id,NEW.target_id,NEW.auth_class,NEW.authority_class,
    NEW.credential_kind,NEW.credential_subject_id,NEW.credential_key_version,
    NEW.credential_digest,NULL,NEW.policy_proofs_json,NEW.entitlement_kind,
    NEW.entitlement_subject_id,NEW.entitlement_revision,
    NEW.entitlement_expires_at_ms,NEW.authority_binding_digest,
    NEW.state,NEW.created_at_ms,NEW.completed_at_ms
  );
END;

CREATE TRIGGER legacy_protected_media_parent_registry_state_v1
AFTER UPDATE OF state,completed_at_ms ON legacy_protected_media_receipts_v1
BEGIN
  UPDATE legacy_protected_effect_parent_registry_v1
  SET state=NEW.state,completed_at_ms=NEW.completed_at_ms
  WHERE parent_family='protected_media' AND parent_receipt_id=NEW.receipt_id;
END;

CREATE TABLE legacy_protected_media_execution_outbox_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_media_receipts_v1(receipt_id) ON DELETE RESTRICT,
  executor_kind TEXT NOT NULL CHECK (
    executor_kind IN ('gstreamer','provider','control_plane')
  ),
  descriptor_json TEXT NOT NULL CHECK (
    json_valid(descriptor_json) AND json_type(descriptor_json) = 'object'
    AND length(descriptor_json) <= 262144
  ),
  descriptor_digest TEXT NOT NULL CHECK (
    length(descriptor_digest)=64 AND descriptor_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL DEFAULT 'pending_execution_evidence' CHECK (
    state IN ('pending_execution_evidence','verified','dead_letter')
  ),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER,
  CHECK (
    (state='pending_execution_evidence' AND completed_at_ms IS NULL)
    OR (state IN ('verified','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_protected_media_outbox_pending_v1
  ON legacy_protected_media_execution_outbox_v1(state,executor_kind,created_at_ms,receipt_id);

-- Natural read/mutation claims coalesce across generation boundaries. Pending
-- work remains reachable indefinitely; only a terminal claim older than the
-- 15-minute sealed-terminal window may roll to a replacement receipt.
CREATE TABLE legacy_protected_media_generated_replay_claims_v1 (
  source_operation_id TEXT NOT NULL,
  principal_digest TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  receipt_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_protected_media_receipts_v1(receipt_id) ON DELETE RESTRICT,
  claimed_at_ms INTEGER NOT NULL CHECK (claimed_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY(source_operation_id,principal_digest,request_digest)
);
CREATE TRIGGER legacy_protected_media_generated_receipt_gate_v1
BEFORE INSERT ON legacy_protected_media_receipts_v1
WHEN NEW.replay_origin='natural' AND EXISTS (
  SELECT 1 FROM legacy_protected_media_generated_replay_claims_v1 claim
  JOIN legacy_protected_media_receipts_v1 prior ON prior.receipt_id=claim.receipt_id
  WHERE claim.source_operation_id=NEW.source_operation_id
    AND claim.principal_digest=NEW.principal_digest
    AND claim.request_digest=NEW.request_digest
    AND (prior.state='pending_execution_evidence' OR prior.completed_at_ms IS NULL
      OR prior.completed_at_ms > NEW.created_at_ms - 900000)
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_generated_replay_claimed_v1'); END;
CREATE TRIGGER legacy_protected_media_generated_claim_insert_gate_v1
BEFORE INSERT ON legacy_protected_media_generated_replay_claims_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_media_receipts_v1 receipt
  WHERE receipt.receipt_id=NEW.receipt_id
    AND receipt.source_operation_id=NEW.source_operation_id
    AND receipt.principal_digest=NEW.principal_digest
    AND receipt.request_digest=NEW.request_digest
    AND receipt.replay_origin='natural'
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_generated_replay_invalid_v1'); END;
CREATE TRIGGER legacy_protected_media_generated_claim_update_gate_v1
BEFORE UPDATE ON legacy_protected_media_generated_replay_claims_v1
WHEN NOT (
  OLD.source_operation_id=NEW.source_operation_id
  AND OLD.principal_digest=NEW.principal_digest
  AND OLD.request_digest=NEW.request_digest
  AND OLD.receipt_id<>NEW.receipt_id
  AND NEW.claimed_at_ms>=OLD.claimed_at_ms
  AND EXISTS (SELECT 1 FROM legacy_protected_media_receipts_v1 prior
    WHERE prior.receipt_id=OLD.receipt_id AND prior.state IN ('verified','dead_letter')
      AND prior.completed_at_ms IS NOT NULL
      AND prior.completed_at_ms<=NEW.claimed_at_ms-900000)
  AND EXISTS (SELECT 1 FROM legacy_protected_media_receipts_v1 replacement
    WHERE replacement.receipt_id=NEW.receipt_id
      AND replacement.source_operation_id=NEW.source_operation_id
      AND replacement.principal_digest=NEW.principal_digest
      AND replacement.request_digest=NEW.request_digest
      AND replacement.replay_origin='natural')
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_generated_replay_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_generated_claim_no_delete_v1
BEFORE DELETE ON legacy_protected_media_generated_replay_claims_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_generated_replay_immutable_v1'); END;

-- Executors and leases are provisioned/issued by a separate control identity.
-- Request handlers cannot mint either one and therefore cannot self-attest.
CREATE TABLE legacy_protected_media_executors_v1 (
  executor_id TEXT PRIMARY KEY NOT NULL CHECK (length(executor_id) BETWEEN 1 AND 255),
  executor_kind TEXT NOT NULL CHECK (
    executor_kind IN ('gstreamer','provider','control_plane')
  ),
  identity_digest TEXT NOT NULL CHECK (
    length(identity_digest)=64 AND identity_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('active','disabled'))
);
CREATE TABLE legacy_protected_media_executor_leases_v1 (
  lease_id TEXT PRIMARY KEY NOT NULL CHECK (length(lease_id)=36),
  receipt_id TEXT NOT NULL REFERENCES legacy_protected_media_receipts_v1(receipt_id)
    ON DELETE RESTRICT,
  executor_id TEXT NOT NULL REFERENCES legacy_protected_media_executors_v1(executor_id)
    ON DELETE RESTRICT,
  request_digest TEXT NOT NULL,
  outbox_descriptor_digest TEXT NOT NULL,
  authority_binding_digest TEXT NOT NULL,
  leased_at_ms INTEGER NOT NULL,
  lease_expires_at_ms INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('active','consumed','expired')),
  CHECK (lease_expires_at_ms>leased_at_ms),
  UNIQUE(receipt_id,lease_id)
);
CREATE UNIQUE INDEX legacy_protected_media_one_active_lease_v1
  ON legacy_protected_media_executor_leases_v1(receipt_id) WHERE state='active';

CREATE TABLE legacy_protected_media_execution_evidence_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_media_receipts_v1(receipt_id) ON DELETE RESTRICT,
  lease_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_protected_media_executor_leases_v1(lease_id) ON DELETE RESTRICT,
  executor_id TEXT NOT NULL REFERENCES legacy_protected_media_executors_v1(executor_id)
    ON DELETE RESTRICT,
  request_digest TEXT NOT NULL,
  outbox_descriptor_digest TEXT NOT NULL,
  authority_binding_digest TEXT NOT NULL,
  execution_evidence_digest TEXT NOT NULL CHECK (
    length(execution_evidence_digest)=64
    AND execution_evidence_digest NOT GLOB '*[^0-9a-f]*'
  ),
  provider_evidence_digest TEXT CHECK (
    provider_evidence_digest IS NULL OR (
      length(provider_evidence_digest)=64
      AND provider_evidence_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  terminal_kind TEXT NOT NULL CHECK (
    terminal_kind IN ('json','redirect','binary','event_stream')
  ),
  sealed_terminal_ref TEXT NOT NULL CHECK (
    length(sealed_terminal_ref)=85
    AND substr(sealed_terminal_ref,1,21)='frame-pm-terminal-v1:'
    AND substr(sealed_terminal_ref,22) NOT GLOB '*[^0-9a-f]*'
  ),
  sealed_terminal_digest TEXT NOT NULL CHECK (
    length(sealed_terminal_digest)=64
    AND sealed_terminal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  terminal_expires_at_ms INTEGER NOT NULL,
  verified_at_ms INTEGER NOT NULL,
  CHECK (terminal_expires_at_ms>verified_at_ms
    AND terminal_expires_at_ms<=verified_at_ms+900000)
);

CREATE TRIGGER legacy_protected_media_evidence_gate_v1
BEFORE INSERT ON legacy_protected_media_execution_evidence_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_media_receipts_v1 receipt
  JOIN legacy_protected_media_execution_outbox_v1 outbox ON outbox.receipt_id=receipt.receipt_id
  JOIN legacy_protected_media_live_authority_v1 live ON live.receipt_id=receipt.receipt_id
  JOIN legacy_protected_media_executor_leases_v1 lease ON lease.receipt_id=receipt.receipt_id
  JOIN legacy_protected_media_executors_v1 executor ON executor.executor_id=lease.executor_id
  WHERE receipt.receipt_id=NEW.receipt_id
    AND receipt.request_digest=NEW.request_digest
    AND receipt.authority_binding_digest=NEW.authority_binding_digest
    AND receipt.terminal_kind=NEW.terminal_kind
    AND receipt.executor_kind=executor.executor_kind
    AND outbox.executor_kind=executor.executor_kind
    AND outbox.descriptor_digest=NEW.outbox_descriptor_digest
    AND lease.lease_id=NEW.lease_id AND lease.executor_id=NEW.executor_id
    AND lease.request_digest=NEW.request_digest
    AND lease.outbox_descriptor_digest=NEW.outbox_descriptor_digest
    AND lease.authority_binding_digest=NEW.authority_binding_digest
    AND lease.state='active' AND executor.state='active'
    AND NEW.verified_at_ms BETWEEN lease.leased_at_ms AND lease.lease_expires_at_ms
    AND live.authority_expires_at_ms>NEW.verified_at_ms
    AND receipt.state='pending_execution_evidence'
    AND outbox.state='pending_execution_evidence'
    AND (receipt.provider_required=0 OR NEW.provider_evidence_digest IS NOT NULL)
    AND (receipt.operation_kind<>'workflow' OR EXISTS (
      SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
      JOIN legacy_protected_effect_parent_edges_v1 edge
        ON edge.parent_family=parent.parent_family
       AND edge.parent_operation_id=parent.source_operation_id
       AND edge.child_family='protected_media'
       AND edge.child_operation_id=receipt.source_operation_id
      WHERE parent.parent_family=receipt.parent_family
        AND parent.parent_receipt_id=receipt.parent_receipt_id
        AND parent.request_digest=receipt.parent_request_digest
        AND parent.state<>'dead_letter'
        AND parent.actor_id IS receipt.actor_id
        AND parent.tenant_id IS receipt.tenant_id
        AND (
          (edge.target_binding_rule='same' AND parent.target_id IS receipt.target_id)
          OR (edge.target_binding_rule='child_derived' AND receipt.target_id IS NOT NULL)
        )
        AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
    ))
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_evidence_invalid_v1'); END;

CREATE TRIGGER legacy_protected_media_verified_gate_v1
BEFORE UPDATE OF state ON legacy_protected_media_receipts_v1
WHEN NEW.state='verified' AND NOT (
  EXISTS (
    SELECT 1 FROM legacy_protected_media_execution_evidence_v1 evidence
    WHERE evidence.receipt_id=NEW.receipt_id
      AND evidence.request_digest=NEW.request_digest
      AND evidence.authority_binding_digest=NEW.authority_binding_digest
  )
  AND EXISTS (
    SELECT 1 FROM legacy_protected_media_execution_outbox_v1 outbox
    WHERE outbox.receipt_id=NEW.receipt_id AND outbox.state='verified'
      AND outbox.completed_at_ms=NEW.completed_at_ms
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_evidence_required_v1'); END;

-- One admitted evidence row consumes its independent lease and atomically
-- closes both durable work records. There is no interval in which a verified
-- terminal can be replayed while the execution outbox remains pending.
CREATE TRIGGER legacy_protected_media_evidence_finalize_v1
AFTER INSERT ON legacy_protected_media_execution_evidence_v1
BEGIN
  UPDATE legacy_protected_media_executor_leases_v1
  SET state='consumed'
  WHERE lease_id=NEW.lease_id AND receipt_id=NEW.receipt_id AND state='active';
  UPDATE legacy_protected_media_execution_outbox_v1
  SET state='verified',completed_at_ms=NEW.verified_at_ms
  WHERE receipt_id=NEW.receipt_id AND state='pending_execution_evidence';
  UPDATE legacy_protected_media_receipts_v1
  SET state='verified',completed_at_ms=NEW.verified_at_ms
  WHERE receipt_id=NEW.receipt_id AND state='pending_execution_evidence';
END;

CREATE TRIGGER legacy_protected_media_receipt_immutable_v1
BEFORE UPDATE ON legacy_protected_media_receipts_v1
WHEN NOT (
  OLD.receipt_id=NEW.receipt_id AND OLD.source_operation_id=NEW.source_operation_id
  AND OLD.operation_kind=NEW.operation_kind AND OLD.method=NEW.method
  AND OLD.surface_path=NEW.surface_path AND OLD.auth_class=NEW.auth_class
  AND OLD.authority_class=NEW.authority_class AND OLD.principal_digest=NEW.principal_digest
  AND OLD.actor_id IS NEW.actor_id AND OLD.tenant_id IS NEW.tenant_id
  AND OLD.credential_kind=NEW.credential_kind
  AND OLD.credential_subject_id=NEW.credential_subject_id
  AND OLD.credential_key_version=NEW.credential_key_version
  AND OLD.credential_digest=NEW.credential_digest
  AND OLD.policy_proofs_json=NEW.policy_proofs_json
  AND OLD.entitlement_kind IS NEW.entitlement_kind
  AND OLD.entitlement_subject_id IS NEW.entitlement_subject_id
  AND OLD.entitlement_revision IS NEW.entitlement_revision
  AND OLD.entitlement_expires_at_ms IS NEW.entitlement_expires_at_ms
  AND OLD.target_id IS NEW.target_id
  AND OLD.authority_binding_digest=NEW.authority_binding_digest
  AND OLD.parent_family IS NEW.parent_family
  AND OLD.parent_receipt_id IS NEW.parent_receipt_id
  AND OLD.parent_request_digest IS NEW.parent_request_digest
  AND OLD.parent_authority_binding_digest IS NEW.parent_authority_binding_digest
  AND OLD.execution_key_digest=NEW.execution_key_digest
  AND OLD.replay_origin=NEW.replay_origin AND OLD.idempotency_mode=NEW.idempotency_mode
  AND OLD.request_digest=NEW.request_digest AND OLD.payload_digest=NEW.payload_digest
  AND OLD.request_descriptor_json=NEW.request_descriptor_json
  AND OLD.sealed_request_ref IS NEW.sealed_request_ref
  AND OLD.sealed_request_digest IS NEW.sealed_request_digest
  AND OLD.terminal_kind=NEW.terminal_kind AND OLD.executor_kind=NEW.executor_kind
  AND OLD.provider_required=NEW.provider_required AND OLD.created_at_ms=NEW.created_at_ms
  AND OLD.state='pending_execution_evidence' AND NEW.state IN ('verified','dead_letter')
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_receipt_no_delete_v1
BEFORE DELETE ON legacy_protected_media_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_outbox_update_gate_v1
BEFORE UPDATE ON legacy_protected_media_execution_outbox_v1
WHEN NOT (
  OLD.receipt_id=NEW.receipt_id AND OLD.executor_kind=NEW.executor_kind
  AND OLD.descriptor_json=NEW.descriptor_json
  AND OLD.descriptor_digest=NEW.descriptor_digest
  AND OLD.created_at_ms=NEW.created_at_ms
  AND OLD.state='pending_execution_evidence'
  AND NEW.state IN ('pending_execution_evidence','verified','dead_letter')
  AND NEW.attempt_count BETWEEN OLD.attempt_count AND 1000000
  AND (
    (NEW.state='pending_execution_evidence' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state IN ('verified','dead_letter') AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_outbox_no_delete_v1
BEFORE DELETE ON legacy_protected_media_execution_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_executor_update_gate_v1
BEFORE UPDATE ON legacy_protected_media_executors_v1
WHEN NOT (
  OLD.executor_id=NEW.executor_id AND OLD.executor_kind=NEW.executor_kind
  AND OLD.identity_digest=NEW.identity_digest
  AND OLD.state='active' AND NEW.state='disabled'
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_executor_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_executor_no_delete_v1
BEFORE DELETE ON legacy_protected_media_executors_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_executor_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_lease_update_gate_v1
BEFORE UPDATE ON legacy_protected_media_executor_leases_v1
WHEN NOT (
  OLD.lease_id=NEW.lease_id AND OLD.receipt_id=NEW.receipt_id
  AND OLD.executor_id=NEW.executor_id AND OLD.request_digest=NEW.request_digest
  AND OLD.outbox_descriptor_digest=NEW.outbox_descriptor_digest
  AND OLD.authority_binding_digest=NEW.authority_binding_digest
  AND OLD.leased_at_ms=NEW.leased_at_ms
  AND OLD.lease_expires_at_ms=NEW.lease_expires_at_ms
  AND OLD.state='active' AND NEW.state IN ('consumed','expired')
)
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_lease_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_lease_no_delete_v1
BEFORE DELETE ON legacy_protected_media_executor_leases_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_lease_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_evidence_immutable_v1
BEFORE UPDATE ON legacy_protected_media_execution_evidence_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_protected_media_evidence_no_delete_v1
BEFORE DELETE ON legacy_protected_media_execution_evidence_v1
BEGIN SELECT RAISE(ABORT, 'frame_protected_media_evidence_immutable_v1'); END;
