PRAGMA foreign_keys = ON;

-- Named endpoint digests are operator-provisioned and rotation-safe. Request
-- handlers may verify the environment secret, but a receipt is live only
-- while the exact digest/version is also admitted here.
CREATE TABLE legacy_protected_integration_signed_authorities_v1 (
  credential_subject_id TEXT PRIMARY KEY NOT NULL CHECK (
    credential_subject_id='media-server-webhook.endpoint.v1'
  ),
  credential_kind TEXT NOT NULL CHECK (credential_kind='signed_endpoint'),
  credential_key_version INTEGER NOT NULL CHECK (
    credential_key_version BETWEEN 1 AND 65535
  ),
  credential_digest TEXT NOT NULL CHECK (
    length(credential_digest)=64 AND credential_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('active','disabled')),
  expires_at_ms INTEGER CHECK (
    expires_at_ms IS NULL OR expires_at_ms BETWEEN 1 AND 9007199254740991
  )
);

CREATE TABLE legacy_protected_integration_receipts_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL CHECK (length(receipt_id)=36),
  source_operation_id TEXT NOT NULL CHECK (
    length(source_operation_id)=23 AND source_operation_id LIKE 'cap-v1-%'
  ),
  operation_kind TEXT NOT NULL CHECK (
    operation_kind IN ('route','rpc','server_action','workflow')
  ),
  method TEXT NOT NULL CHECK (
    method IN ('GET','PATCH','POST','DELETE','RPC','ACTION','WORKFLOW')
  ),
  surface_path TEXT NOT NULL CHECK (length(surface_path) BETWEEN 1 AND 512),
  auth_class TEXT NOT NULL CHECK (auth_class IN (
    'session','session_or_api_key','anonymous_or_session_or_api_key',
    'signed_state','public','public_or_session','parent_receipt','signed_webhook'
  )),
  authority_class TEXT NOT NULL CHECK (authority_class IN (
    'session','session_or_organization_member','session_or_organization_owner',
    'organization_member','organization_manager','organization_owner',
    'space_manager','video_viewer','public','signed_state',
    'signed_state_or_organization_owner','parent_receipt','signed_webhook'
  )),
  provider_kind TEXT NOT NULL CHECK (length(provider_kind) BETWEEN 1 AND 96),
  principal_digest TEXT NOT NULL CHECK (
    length(principal_digest)=64 AND principal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  actor_id TEXT CHECK (actor_id IS NULL OR length(actor_id) BETWEEN 1 AND 255),
  tenant_id TEXT CHECK (tenant_id IS NULL OR length(tenant_id) BETWEEN 1 AND 512),
  target_id TEXT CHECK (target_id IS NULL OR length(target_id) BETWEEN 1 AND 512),
  tenant_domain TEXT NOT NULL CHECK (
    tenant_domain IN ('none','organization','external')
  ),
  target_domain TEXT NOT NULL CHECK (
    target_domain IN ('none','video','space','external')
  ),
  legacy_tenant_id TEXT CHECK (
    legacy_tenant_id IS NULL OR length(legacy_tenant_id) BETWEEN 1 AND 512
  ),
  legacy_target_id TEXT CHECK (
    legacy_target_id IS NULL OR length(legacy_target_id) BETWEEN 1 AND 512
  ),
  legacy_workflow_actor_id TEXT CHECK (
    legacy_workflow_actor_id IS NULL OR (
      length(legacy_workflow_actor_id)=15
      AND legacy_workflow_actor_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  legacy_workflow_cap_tenant_id TEXT CHECK (
    legacy_workflow_cap_tenant_id IS NULL OR (
      length(legacy_workflow_cap_tenant_id)=15
      AND legacy_workflow_cap_tenant_id
        NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  workflow_raw_file_key TEXT CHECK (
    workflow_raw_file_key IS NULL OR (
      length(workflow_raw_file_key) BETWEEN 35 AND 512
      AND workflow_raw_file_key NOT LIKE '%..%'
      AND workflow_raw_file_key NOT LIKE '%//%'
      AND workflow_raw_file_key NOT LIKE '%\%'
    )
  ),
  credential_kind TEXT NOT NULL CHECK (credential_kind IN (
    'none','session_token','api_key','signed_state','signed_endpoint'
  )),
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
    AND json_array_length(policy_proofs_json) BETWEEN 0 AND 8
    AND length(policy_proofs_json)<=65536
  ),
  entitlement_kind TEXT CHECK (entitlement_kind IS NULL OR entitlement_kind IN (
    'cap_internal','pro','subscription_read','subscription_manage','ai_owner'
  )),
  entitlement_subject_id TEXT CHECK (
    entitlement_subject_id IS NULL OR length(entitlement_subject_id) BETWEEN 1 AND 255
  ),
  entitlement_revision INTEGER CHECK (
    entitlement_revision IS NULL
    OR entitlement_revision BETWEEN 0 AND 9007199254740991
  ),
  entitlement_expires_at_ms INTEGER CHECK (
    entitlement_expires_at_ms IS NULL
    OR entitlement_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  conditional_bindings_json TEXT NOT NULL CHECK (
    json_valid(conditional_bindings_json)
    AND json_type(conditional_bindings_json)='array'
    AND json_array_length(conditional_bindings_json) BETWEEN 0 AND 8
    AND length(conditional_bindings_json)<=65536
  ),
  authority_binding_digest TEXT NOT NULL CHECK (
    length(authority_binding_digest)=64
    AND authority_binding_digest NOT GLOB '*[^0-9a-f]*'
  ),
  parent_family TEXT CHECK (
    parent_family IS NULL OR parent_family IN ('protected_integrations','protected_media')
  ),
  parent_receipt_id TEXT CHECK (
    parent_receipt_id IS NULL OR length(parent_receipt_id)=36
  ),
  parent_request_digest TEXT CHECK (
    parent_request_digest IS NULL OR (
      length(parent_request_digest)=64
      AND parent_request_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  parent_authority_binding_digest TEXT CHECK (
    parent_authority_binding_digest IS NULL OR (
      length(parent_authority_binding_digest)=64
      AND parent_authority_binding_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  replay_key_digest TEXT NOT NULL CHECK (
    length(replay_key_digest)=64 AND replay_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  replay_origin TEXT NOT NULL CHECK (replay_origin IN ('generated','natural')),
  idempotency_mode TEXT NOT NULL CHECK (
    idempotency_mode IN ('required','optional','forbidden')
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest)=64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  redacted_request_json TEXT NOT NULL CHECK (
    json_valid(redacted_request_json) AND json_type(redacted_request_json)='object'
    AND length(redacted_request_json)<=8388608
  ),
  sealed_request_ref TEXT NOT NULL CHECK (
    length(sealed_request_ref)=84
    AND substr(sealed_request_ref,1,20)='frame-pi-request-v1:'
    AND substr(sealed_request_ref,21) NOT GLOB '*[^0-9a-f]*'
  ),
  sealed_request_digest TEXT NOT NULL CHECK (
    length(sealed_request_digest)=64
    AND sealed_request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  transport_body_digest TEXT CHECK (
    transport_body_digest IS NULL OR (
      length(transport_body_digest)=64
      AND transport_body_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  terminal_kind TEXT NOT NULL CHECK (terminal_kind IN ('http','json','workflow')),
  conditional_duration_seconds INTEGER CHECK (
    conditional_duration_seconds IS NULL
    OR conditional_duration_seconds BETWEEN 0 AND 9007199254740991
  ),
  conditional_password_requested INTEGER NOT NULL CHECK (
    conditional_password_requested IN (0,1)
  ),
  conditional_pro_settings_requested INTEGER NOT NULL CHECK (
    conditional_pro_settings_requested IN (0,1)
  ),
  conditional_public_requested INTEGER NOT NULL CHECK (
    conditional_public_requested IN (0,1)
  ),
  seat_quantity INTEGER CHECK (seat_quantity IS NULL OR seat_quantity BETWEEN 1 AND 500),
  state TEXT NOT NULL DEFAULT 'pending_provider_evidence' CHECK (
    state IN ('pending_provider_evidence','verified','dead_letter')
  ),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  UNIQUE(source_operation_id,principal_digest,replay_key_digest),
  CHECK (
    (source_operation_id='cap-v1-b9fcb0fbd25b2234'
      AND legacy_workflow_actor_id IS NOT NULL
      AND legacy_workflow_cap_tenant_id IS NULL
      AND workflow_raw_file_key IS NOT NULL)
    OR (source_operation_id IN (
        'cap-v1-bd1b9d67380624f7','cap-v1-f0a00e93ab606a52'
      )
      AND legacy_workflow_actor_id IS NOT NULL
      AND legacy_workflow_cap_tenant_id IS NOT NULL
      AND workflow_raw_file_key IS NULL)
    OR (source_operation_id NOT IN (
        'cap-v1-b9fcb0fbd25b2234','cap-v1-bd1b9d67380624f7',
        'cap-v1-f0a00e93ab606a52'
      ) AND legacy_workflow_actor_id IS NULL
      AND legacy_workflow_cap_tenant_id IS NULL
      AND workflow_raw_file_key IS NULL)
  ),
  CHECK (
    (credential_kind='none' AND credential_subject_id IS NULL
      AND credential_key_version IS NULL AND credential_digest IS NULL
      AND credential_expires_at_ms IS NULL)
    OR (credential_kind='session_token' AND credential_subject_id IS NOT NULL
      AND length(credential_subject_id)=36 AND credential_key_version BETWEEN 1 AND 65535
      AND credential_digest IS NOT NULL AND credential_expires_at_ms IS NULL)
    OR (credential_kind='api_key' AND credential_subject_id IS NOT NULL
      AND credential_key_version IS NULL AND credential_digest IS NOT NULL
      AND credential_expires_at_ms IS NULL)
    OR (credential_kind='signed_state'
      AND credential_subject_id='google-drive-oauth-state.v1'
      AND credential_key_version=1 AND credential_digest IS NOT NULL
      AND credential_expires_at_ms IS NOT NULL)
    OR (credential_kind='signed_endpoint'
      AND credential_subject_id='media-server-webhook.endpoint.v1'
      AND credential_key_version=1 AND credential_digest IS NOT NULL
      AND credential_expires_at_ms IS NULL)
  ),
  CHECK (
    (entitlement_kind IS NULL AND entitlement_subject_id IS NULL
      AND entitlement_revision IS NULL AND entitlement_expires_at_ms IS NULL)
    OR (entitlement_kind IS NOT NULL AND entitlement_subject_id IS NOT NULL
      AND entitlement_revision IS NOT NULL)
  ),
  CHECK (
    (operation_kind='workflow' AND replay_origin='natural'
      AND parent_family IS NOT NULL AND parent_receipt_id IS NOT NULL
      AND parent_request_digest IS NOT NULL
      AND parent_authority_binding_digest IS NOT NULL)
    OR (operation_kind<>'workflow' AND parent_family IS NULL
      AND parent_receipt_id IS NULL AND parent_request_digest IS NULL
      AND parent_authority_binding_digest IS NULL)
  ),
  CHECK (
    (auth_class='signed_webhook' AND replay_origin='natural'
      AND credential_kind='signed_endpoint' AND transport_body_digest IS NOT NULL)
    OR auth_class<>'signed_webhook'
  ),
  CHECK (
    (auth_class='public' AND credential_kind='none' AND actor_id IS NULL)
    OR (auth_class='signed_state' AND credential_kind='signed_state' AND actor_id IS NOT NULL)
    OR (auth_class='signed_webhook' AND credential_kind='signed_endpoint' AND actor_id IS NULL)
    OR (auth_class='session' AND credential_kind='session_token' AND actor_id IS NOT NULL)
    OR (auth_class='session_or_api_key' AND credential_kind IN ('session_token','api_key')
      AND actor_id IS NOT NULL)
    OR (auth_class='anonymous_or_session_or_api_key' AND (
      (credential_kind='none' AND actor_id IS NULL)
      OR (credential_kind IN ('session_token','api_key') AND actor_id IS NOT NULL)))
    OR (auth_class='public_or_session' AND (
      (credential_kind='none' AND actor_id IS NULL)
      OR (credential_kind='session_token' AND actor_id IS NOT NULL)))
    OR auth_class='parent_receipt'
  ),
  CHECK (
    (terminal_kind='http' AND operation_kind='route')
    OR (terminal_kind='json' AND operation_kind IN ('rpc','server_action'))
    OR (terminal_kind='workflow' AND operation_kind='workflow')
  ),
  CHECK (
    (state='pending_provider_evidence' AND completed_at_ms IS NULL)
    OR (state IN ('verified','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_protected_integration_receipts_target_v1
  ON legacy_protected_integration_receipts_v1(tenant_id,target_id,created_at_ms,receipt_id);
CREATE INDEX legacy_protected_integration_receipts_parent_v1
  ON legacy_protected_integration_receipts_v1(parent_family,parent_receipt_id,receipt_id);
CREATE UNIQUE INDEX legacy_protected_integration_natural_replay_unique_v1
  ON legacy_protected_integration_receipts_v1(source_operation_id,replay_key_digest)
  WHERE replay_origin='natural';

CREATE TABLE legacy_protected_integration_generated_replay_claims_v1 (
  source_operation_id TEXT NOT NULL,
  principal_digest TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  receipt_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_protected_integration_receipts_v1(receipt_id) ON DELETE RESTRICT,
  claimed_at_ms INTEGER NOT NULL CHECK (
    claimed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY(source_operation_id,principal_digest,request_digest)
);
CREATE TRIGGER legacy_protected_integration_generated_receipt_claim_gate_v1
BEFORE INSERT ON legacy_protected_integration_receipts_v1
WHEN NEW.replay_origin='generated' AND EXISTS (
  SELECT 1 FROM legacy_protected_integration_generated_replay_claims_v1 claim
  JOIN legacy_protected_integration_receipts_v1 prior ON prior.receipt_id=claim.receipt_id
  WHERE claim.source_operation_id=NEW.source_operation_id
    AND claim.principal_digest=NEW.principal_digest
    AND claim.request_digest=NEW.request_digest
    AND (prior.state='pending_provider_evidence' OR prior.completed_at_ms IS NULL
      OR prior.completed_at_ms>NEW.created_at_ms-900000)
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_generated_replay_claimed_v1'); END;
CREATE TRIGGER legacy_protected_integration_generated_claim_insert_gate_v1
BEFORE INSERT ON legacy_protected_integration_generated_replay_claims_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_receipts_v1 receipt
  WHERE receipt.receipt_id=NEW.receipt_id
    AND receipt.source_operation_id=NEW.source_operation_id
    AND receipt.principal_digest=NEW.principal_digest
    AND receipt.request_digest=NEW.request_digest
    AND receipt.replay_origin='generated'
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_generated_replay_invalid_v1'); END;
CREATE TRIGGER legacy_protected_integration_generated_claim_update_gate_v1
BEFORE UPDATE ON legacy_protected_integration_generated_replay_claims_v1
WHEN NOT (
  OLD.source_operation_id=NEW.source_operation_id
  AND OLD.principal_digest=NEW.principal_digest
  AND OLD.request_digest=NEW.request_digest
  AND OLD.receipt_id<>NEW.receipt_id AND NEW.claimed_at_ms>=OLD.claimed_at_ms
  AND EXISTS (SELECT 1 FROM legacy_protected_integration_receipts_v1 prior
    WHERE prior.receipt_id=OLD.receipt_id AND prior.state IN ('verified','dead_letter')
      AND prior.completed_at_ms IS NOT NULL
      AND prior.completed_at_ms<=NEW.claimed_at_ms-900000)
  AND EXISTS (SELECT 1 FROM legacy_protected_integration_receipts_v1 replacement
    WHERE replacement.receipt_id=NEW.receipt_id
      AND replacement.source_operation_id=NEW.source_operation_id
      AND replacement.principal_digest=NEW.principal_digest
      AND replacement.request_digest=NEW.request_digest
      AND replacement.replay_origin='generated')
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_generated_replay_immutable_v1'); END;
CREATE TRIGGER legacy_protected_integration_generated_claim_no_delete_v1
BEFORE DELETE ON legacy_protected_integration_generated_replay_claims_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_generated_replay_immutable_v1'); END;

CREATE TABLE legacy_protected_integration_outbox_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_integration_receipts_v1(receipt_id) ON DELETE RESTRICT,
  provider_kind TEXT NOT NULL CHECK (length(provider_kind) BETWEEN 1 AND 96),
  payload_json TEXT NOT NULL CHECK (
    json_valid(payload_json) AND json_type(payload_json)='object'
    AND length(payload_json)<=8388608
  ),
  payload_digest TEXT NOT NULL CHECK (
    length(payload_digest)=64 AND payload_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL DEFAULT 'pending_provider_evidence' CHECK (
    state IN ('pending_provider_evidence','verified','dead_letter')
  ),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (
    attempt_count BETWEEN 0 AND 1000000
  ),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  ),
  completed_at_ms INTEGER,
  CHECK (
    (state='pending_provider_evidence' AND completed_at_ms IS NULL)
    OR (state IN ('verified','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_protected_integration_outbox_pending_v1
  ON legacy_protected_integration_outbox_v1(state,provider_kind,created_at_ms,receipt_id);

-- Exact VideosPolicy projection for getVideoStatus. A password proof is tied
-- to the current video/space revision; video password wins, otherwise the
-- lowest stable id among still-associated protected spaces is the only valid
-- proof.
CREATE VIEW legacy_protected_integration_video_policy_live_v1 AS
SELECT receipt.receipt_id
FROM legacy_protected_integration_receipts_v1 receipt
JOIN videos video ON video.id=receipt.target_id
WHERE receipt.authority_class='video_viewer' AND video.deleted_at_ms IS NULL
  AND json_array_length(receipt.policy_proofs_json)=1
  AND json_extract(receipt.policy_proofs_json,'$[0].target_id')=video.id
  AND (
    (video.owner_id=receipt.actor_id
      AND json_extract(receipt.policy_proofs_json,'$[0].kind')='owner_bypass'
      AND json_extract(receipt.policy_proofs_json,'$[0].subject_id')=video.id
      AND json_extract(receipt.policy_proofs_json,'$[0].revision')=video.legacy_property_revision)
    OR (
      (
        EXISTS (SELECT 1 FROM organization_members member
          WHERE member.organization_id=video.organization_id
            AND member.user_id=receipt.actor_id AND member.state='active')
        OR EXISTS (SELECT 1 FROM space_videos placement
          JOIN space_members member ON member.space_id=placement.space_id
          JOIN spaces space ON space.id=placement.space_id
          WHERE placement.video_id=video.id AND member.user_id=receipt.actor_id
            AND space.deleted_at_ms IS NULL)
        OR (video.legacy_public=1 AND (
          COALESCE(TRIM((SELECT legacy_allowed_email_restriction
            FROM organizations WHERE id=video.organization_id)),'')=''
          OR (receipt.actor_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
              AND (instr(','||lower(replace((SELECT legacy_allowed_email_restriction
                FROM organizations WHERE id=video.organization_id),' ',''))||',',
                ','||lower(actor.email)||',')>0
               OR instr(','||lower(replace((SELECT legacy_allowed_email_restriction
                FROM organizations WHERE id=video.organization_id),' ',''))||',',
                ','||lower(substr(actor.email,instr(actor.email,'@')+1))||',')>0)
          ))
        ))
      )
      AND (
        (video.legacy_password_hash IS NOT NULL
          AND json_extract(receipt.policy_proofs_json,'$[0].kind')='video_password'
          AND json_extract(receipt.policy_proofs_json,'$[0].subject_id')=video.id
          AND json_extract(receipt.policy_proofs_json,'$[0].revision')=video.legacy_property_revision)
        OR (video.legacy_password_hash IS NULL AND EXISTS (
          SELECT 1 FROM space_videos placement JOIN spaces space
            ON space.id=placement.space_id
          WHERE placement.video_id=video.id AND space.deleted_at_ms IS NULL
            AND space.legacy_password_hash IS NOT NULL
            AND space.id=(
              SELECT MIN(candidate.id)
              FROM space_videos candidate_placement
              JOIN spaces candidate ON candidate.id=candidate_placement.space_id
              WHERE candidate_placement.video_id=video.id
                AND candidate.deleted_at_ms IS NULL
                AND candidate.legacy_password_hash IS NOT NULL
            )
            AND json_extract(receipt.policy_proofs_json,'$[0].kind')='space_password'
            AND json_extract(receipt.policy_proofs_json,'$[0].subject_id')=space.id
            AND json_extract(receipt.policy_proofs_json,'$[0].revision')=space.legacy_password_revision
        ))
        OR (video.legacy_password_hash IS NULL AND NOT EXISTS (
          SELECT 1 FROM space_videos placement JOIN spaces space
            ON space.id=placement.space_id
          WHERE placement.video_id=video.id AND space.deleted_at_ms IS NULL
            AND space.legacy_password_hash IS NOT NULL
        ) AND json_extract(receipt.policy_proofs_json,'$[0].kind')='unprotected_video_policy'
          AND json_extract(receipt.policy_proofs_json,'$[0].subject_id')=video.id
          AND json_extract(receipt.policy_proofs_json,'$[0].revision')=video.legacy_property_revision)
      )
    )
  );

CREATE VIEW legacy_protected_integration_live_authority_v1 AS
WITH credential_live AS (
  SELECT receipt.receipt_id,
    CASE
      WHEN receipt.credential_kind='none' THEN 9007199254740991
      WHEN receipt.credential_kind='session_token' THEN (
        SELECT MIN(session.idle_expires_at_ms,session.absolute_expires_at_ms)
        FROM auth_sessions_v2 session
        JOIN auth_identities_v2 identity ON identity.user_id=session.user_id
        JOIN users actor ON actor.id=session.user_id
        WHERE session.id=receipt.credential_subject_id
          AND session.user_id=receipt.actor_id
          AND session.token_key_version=receipt.credential_key_version
          AND session.token_digest=receipt.credential_digest
          AND session.state='active' AND session.revoked_at_ms IS NULL
          AND session.session_version=identity.session_version
          AND actor.status='active' AND actor.deleted_at_ms IS NULL
      )
      WHEN receipt.credential_kind='api_key' THEN (
        SELECT COALESCE(key.expires_at_ms,9007199254740991)
        FROM auth_api_keys key JOIN users actor ON actor.id=key.user_id
        WHERE key.id=receipt.credential_subject_id AND key.user_id=receipt.actor_id
          AND key.key_digest=receipt.credential_digest AND key.revoked_at_ms IS NULL
          AND actor.status='active' AND actor.deleted_at_ms IS NULL
      )
      WHEN receipt.credential_kind='signed_state' THEN receipt.credential_expires_at_ms
      WHEN receipt.credential_kind='signed_endpoint' THEN (
        SELECT COALESCE(authority.expires_at_ms,9007199254740991)
        FROM legacy_protected_integration_signed_authorities_v1 authority
        WHERE authority.credential_subject_id=receipt.credential_subject_id
          AND authority.credential_kind=receipt.credential_kind
          AND authority.credential_key_version=receipt.credential_key_version
          AND authority.credential_digest=receipt.credential_digest
          AND authority.state='active'
      )
    END AS credential_expires_at_ms
  FROM legacy_protected_integration_receipts_v1 receipt
),
eligible AS (
  SELECT receipt.*,
    MIN(credential.credential_expires_at_ms,
      COALESCE(receipt.entitlement_expires_at_ms,9007199254740991)) AS authority_expires_at_ms
  FROM legacy_protected_integration_receipts_v1 receipt
  JOIN credential_live credential ON credential.receipt_id=receipt.receipt_id
  WHERE credential.credential_expires_at_ms IS NOT NULL
    AND (
      receipt.tenant_domain<>'organization' OR receipt.legacy_tenant_id IS NULL
      OR EXISTS (SELECT 1 FROM legacy_user_account_organization_ids_v1 alias
        WHERE alias.legacy_organization_id=receipt.legacy_tenant_id
          AND alias.organization_id=receipt.tenant_id)
    )
    AND (
      receipt.target_domain NOT IN ('video','space') OR receipt.legacy_target_id IS NULL
      OR (receipt.target_domain='video' AND EXISTS (
        SELECT 1 FROM legacy_collaboration_video_aliases_v1 alias
        WHERE alias.legacy_video_id=receipt.legacy_target_id
          AND alias.mapped_video_id=receipt.target_id))
      OR (receipt.target_domain='space' AND EXISTS (
        SELECT 1 FROM legacy_library_space_aliases_v1 alias
        WHERE alias.legacy_space_id=receipt.legacy_target_id
          AND alias.space_id=receipt.target_id))
    )
    AND (
      (receipt.source_operation_id='cap-v1-b9fcb0fbd25b2234' AND EXISTS (
        SELECT 1 FROM videos video
        JOIN legacy_collaboration_user_aliases_v1 owner_alias
          ON owner_alias.mapped_user_id=video.owner_id
         AND owner_alias.legacy_user_id=receipt.legacy_workflow_actor_id
        JOIN legacy_protected_effect_parent_registry_v1 parent
          ON parent.parent_family=receipt.parent_family
         AND parent.parent_receipt_id=receipt.parent_receipt_id
         AND parent.request_digest=receipt.parent_request_digest
         AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
         AND parent.state<>'dead_letter'
        WHERE video.id=receipt.target_id AND video.deleted_at_ms IS NULL
          AND receipt.tenant_id IS NOT NULL
          AND video.organization_id=receipt.tenant_id
          AND (
            (parent.source_operation_id='cap-v1-d062d262b013a0cd' AND EXISTS (
              SELECT 1 FROM organization_members member
              WHERE member.organization_id=receipt.tenant_id
                AND member.user_id=video.owner_id AND member.state='active'
            ) AND receipt.workflow_raw_file_key=
              receipt.legacy_workflow_actor_id||'/'||receipt.legacy_target_id||
                '/raw-upload.mp4')
            OR (parent.source_operation_id='cap-v1-cb3fade2af06d6bd'
              AND video.owner_id=receipt.actor_id
              AND receipt.workflow_raw_file_key=
                receipt.legacy_workflow_actor_id||'/'||receipt.legacy_target_id||
                  '/raw-upload.mp4')
            OR (parent.source_operation_id='cap-v1-94a9944ce37fa085'
              AND video.owner_id=receipt.actor_id AND EXISTS (
                SELECT 1 FROM legacy_mobile_cap_uploads_v1 upload
                WHERE upload.mapped_video_id=video.id
                  AND upload.raw_file_key=receipt.workflow_raw_file_key
              ))
          )
      ))
      OR (receipt.source_operation_id='cap-v1-bd1b9d67380624f7' AND EXISTS (
        SELECT 1 FROM legacy_collaboration_user_aliases_v1 actor_alias
        JOIN legacy_user_account_organization_ids_v1 cap_organization
          ON cap_organization.legacy_organization_id=
            receipt.legacy_workflow_cap_tenant_id
        JOIN legacy_user_account_organization_ids_v1 loom_organization
          ON loom_organization.legacy_organization_id=receipt.legacy_tenant_id
        JOIN legacy_protected_effect_parent_registry_v1 parent
          ON parent.parent_family=receipt.parent_family
         AND parent.parent_receipt_id=receipt.parent_receipt_id
         AND parent.request_digest=receipt.parent_request_digest
         AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
         AND parent.source_operation_id='cap-v1-f0a00e93ab606a52'
         AND parent.state<>'dead_letter'
        WHERE actor_alias.legacy_user_id=receipt.legacy_workflow_actor_id
          AND actor_alias.mapped_user_id=receipt.actor_id
          AND parent.actor_id=receipt.actor_id
          AND parent.tenant_id=receipt.tenant_id
          AND cap_organization.organization_id=receipt.tenant_id
          AND loom_organization.organization_id=receipt.tenant_id
      ))
      OR (receipt.source_operation_id='cap-v1-f0a00e93ab606a52' AND EXISTS (
        SELECT 1 FROM legacy_collaboration_user_aliases_v1 loom_actor
        JOIN legacy_user_account_organization_ids_v1 cap_organization
          ON cap_organization.legacy_organization_id=receipt.legacy_tenant_id
        JOIN legacy_user_account_organization_ids_v1 loom_organization
          ON loom_organization.legacy_organization_id=
            receipt.legacy_workflow_cap_tenant_id
        WHERE loom_actor.legacy_user_id=receipt.legacy_workflow_actor_id
          AND loom_actor.mapped_user_id=receipt.actor_id
          AND cap_organization.organization_id=receipt.tenant_id
          AND loom_organization.organization_id=receipt.tenant_id
      ))
      OR receipt.source_operation_id NOT IN (
        'cap-v1-b9fcb0fbd25b2234','cap-v1-bd1b9d67380624f7',
        'cap-v1-f0a00e93ab606a52'
      )
    )
    AND (
      receipt.entitlement_kind IS NULL
      OR (receipt.entitlement_subject_id=receipt.actor_id AND EXISTS (
        SELECT 1 FROM users actor
        WHERE actor.id=receipt.actor_id AND actor.status='active'
          AND actor.deleted_at_ms IS NULL
          AND actor.legacy_user_account_authority_version=receipt.entitlement_revision
          AND (
            (receipt.entitlement_kind='cap_internal' AND lower(actor.email) LIKE '%@cap.so')
            OR (receipt.entitlement_kind='pro' AND (
              actor.legacy_third_party_stripe_subscription_id IS NOT NULL
              OR actor.legacy_stripe_subscription_status IN ('active','trialing','complete','paid')))
            OR (receipt.entitlement_kind='subscription_read'
              AND actor.legacy_stripe_subscription_id IS NOT NULL)
            OR (receipt.entitlement_kind='subscription_manage'
              AND actor.legacy_stripe_subscription_id IS NOT NULL
              AND actor.legacy_stripe_customer_id IS NOT NULL)
          )
      ))
      OR (receipt.entitlement_kind='ai_owner' AND EXISTS (
        SELECT 1 FROM legacy_protected_media_ai_entitlements_v1 entitlement
        WHERE entitlement.user_id=receipt.entitlement_subject_id
          AND entitlement.entitlement_revision=receipt.entitlement_revision
          AND entitlement.state='active'
          AND entitlement.expires_at_ms IS receipt.entitlement_expires_at_ms
      ))
    )
    AND NOT EXISTS (
      SELECT 1 FROM json_each(receipt.conditional_bindings_json) binding
      WHERE NOT CASE json_extract(binding.value,'$.kind')
        WHEN 'video_existing_owner' THEN EXISTS (
          SELECT 1 FROM videos video WHERE video.id=receipt.target_id
            AND video.id=json_extract(binding.value,'$.subject_id')
            AND video.owner_id=receipt.actor_id AND video.deleted_at_ms IS NULL
            AND video.legacy_property_revision=json_extract(binding.value,'$.revision'))
        WHEN 'video_new_organization_member' THEN EXISTS (
          SELECT 1 FROM organizations organization
          LEFT JOIN organization_members member
            ON member.organization_id=organization.id AND member.user_id=receipt.actor_id
              AND member.state='active'
          WHERE organization.id=receipt.tenant_id
            AND organization.id=json_extract(binding.value,'$.subject_id')
            AND organization.status='active'
            AND organization.authority_version=json_extract(binding.value,'$.revision')
            AND (organization.owner_id=receipt.actor_id OR member.user_id IS NOT NULL))
        WHEN 'video_duration_pro' THEN EXISTS (
          SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
            AND actor.id=json_extract(binding.value,'$.subject_id')
            AND actor.legacy_user_account_authority_version=json_extract(binding.value,'$.revision')
            AND (actor.legacy_third_party_stripe_subscription_id IS NOT NULL
              OR actor.legacy_stripe_subscription_status IN ('active','trialing','complete','paid')))
        WHEN 'space_password_pro' THEN EXISTS (
          SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
            AND actor.id=json_extract(binding.value,'$.subject_id')
            AND actor.legacy_user_account_authority_version=json_extract(binding.value,'$.revision')
            AND (actor.legacy_third_party_stripe_subscription_id IS NOT NULL
              OR actor.legacy_stripe_subscription_status IN ('active','trialing','complete','paid')))
        WHEN 'space_settings_pro' THEN EXISTS (
          SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
            AND actor.id=json_extract(binding.value,'$.subject_id')
            AND actor.legacy_user_account_authority_version=json_extract(binding.value,'$.revision')
            AND (actor.legacy_third_party_stripe_subscription_id IS NOT NULL
              OR actor.legacy_stripe_subscription_status IN ('active','trialing','complete','paid')))
        WHEN 'space_publish_owner_pro' THEN EXISTS (
          SELECT 1 FROM organizations organization
          JOIN users owner ON owner.id=organization.owner_id
            AND owner.status='active' AND owner.deleted_at_ms IS NULL
          LEFT JOIN spaces space ON receipt.target_domain='space'
            AND space.id=receipt.target_id AND space.organization_id=organization.id
            AND space.deleted_at_ms IS NULL
          WHERE organization.id=CASE
              WHEN receipt.target_domain='space' THEN space.organization_id
              ELSE receipt.tenant_id
            END
            AND organization.status='active'
            AND owner.id=json_extract(binding.value,'$.subject_id')
            AND owner.legacy_user_account_authority_version=
              json_extract(binding.value,'$.revision')
            AND (owner.legacy_third_party_stripe_subscription_id IS NOT NULL
              OR owner.legacy_stripe_subscription_status IN ('active','trialing','complete','paid')))
        WHEN 'seat_capacity' THEN EXISTS (
          SELECT 1 FROM organizations organization
          WHERE organization.id=receipt.tenant_id
            AND organization.id=json_extract(binding.value,'$.subject_id')
            AND organization.authority_version=json_extract(binding.value,'$.revision')
            AND json_extract(binding.value,'$.value')=receipt.seat_quantity
            AND receipt.seat_quantity>=(SELECT COUNT(*) FROM organization_members member
              WHERE member.organization_id=organization.id
                AND member.state='active' AND (
                  member.has_pro_seat=1 OR (
                    member.user_id=organization.owner_id AND EXISTS (
                      SELECT 1 FROM users owner
                      WHERE owner.id=organization.owner_id
                        AND owner.status='active' AND owner.deleted_at_ms IS NULL
                        AND (owner.legacy_third_party_stripe_subscription_id IS NOT NULL
                          OR owner.legacy_stripe_subscription_status
                            IN ('active','trialing','complete','paid'))
                    )
                  )
                )))
        ELSE 0 END
    )
    AND (receipt.source_operation_id<>'cap-v1-60f863b2cb19353f' OR (
      (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='video_existing_owner')=
        CASE WHEN receipt.target_id IS NOT NULL THEN 1 ELSE 0 END
      AND (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='video_new_organization_member')=
        CASE WHEN receipt.target_id IS NULL THEN 1 ELSE 0 END
      AND (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='video_duration_pro')=
        CASE WHEN COALESCE(receipt.conditional_duration_seconds,0)>300 THEN 1 ELSE 0 END
    ))
    AND (receipt.source_operation_id NOT IN (
        'cap-v1-0c233c1115838206','cap-v1-5e7e4265d65c8365'
      ) OR (
      (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='space_password_pro')=
        receipt.conditional_password_requested
      AND (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='space_settings_pro')=
        receipt.conditional_pro_settings_requested
      AND (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='space_publish_owner_pro')=
        receipt.conditional_public_requested
    ))
    AND (receipt.source_operation_id NOT IN (
        'cap-v1-3a394a2798233b0b','cap-v1-d05af581fbeb145e'
      ) OR (
      (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='space_password_pro')=
        receipt.conditional_password_requested
      AND (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='space_settings_pro')=0
      AND (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='space_publish_owner_pro')=
        CASE WHEN receipt.conditional_public_requested=1 AND EXISTS (
          SELECT 1 FROM spaces WHERE id=receipt.target_id AND is_public=0
        ) THEN 1 ELSE 0 END
    ))
    AND (receipt.source_operation_id NOT IN (
        'cap-v1-aa00fc906599e89c','cap-v1-17470f7df902263e'
      ) OR (
      (SELECT COUNT(*) FROM json_each(receipt.conditional_bindings_json) binding
       WHERE json_extract(binding.value,'$.kind')='seat_capacity')=1
    ))
    AND json_array_length(receipt.conditional_bindings_json)=CASE
      WHEN receipt.source_operation_id='cap-v1-60f863b2cb19353f'
        THEN 1+CASE WHEN COALESCE(receipt.conditional_duration_seconds,0)>300 THEN 1 ELSE 0 END
      WHEN receipt.source_operation_id IN ('cap-v1-0c233c1115838206','cap-v1-5e7e4265d65c8365')
        THEN receipt.conditional_password_requested
          +receipt.conditional_pro_settings_requested
          +receipt.conditional_public_requested
      WHEN receipt.source_operation_id IN ('cap-v1-3a394a2798233b0b','cap-v1-d05af581fbeb145e')
        THEN receipt.conditional_password_requested+CASE
          WHEN receipt.conditional_public_requested=1 AND EXISTS (
            SELECT 1 FROM spaces WHERE id=receipt.target_id AND is_public=0
          ) THEN 1 ELSE 0 END
      WHEN receipt.source_operation_id IN ('cap-v1-aa00fc906599e89c','cap-v1-17470f7df902263e')
        THEN 1
      ELSE 0 END
)
SELECT receipt_id,authority_expires_at_ms FROM eligible receipt
WHERE
  (receipt.authority_class='public')
  OR (receipt.authority_class='signed_webhook' AND EXISTS (
    SELECT 1 FROM legacy_protected_integration_signed_authorities_v1 authority
    WHERE authority.credential_subject_id=receipt.credential_subject_id
      AND authority.credential_key_version=receipt.credential_key_version
      AND authority.credential_digest=receipt.credential_digest
      AND authority.state='active'
  ) AND (receipt.target_id IS NULL OR EXISTS (
    SELECT 1 FROM videos video WHERE video.id=receipt.target_id
      AND video.deleted_at_ms IS NULL)))
  OR (receipt.authority_class IN ('session','signed_state') AND EXISTS (
    SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
      AND actor.status='active' AND actor.deleted_at_ms IS NULL))
  OR (receipt.authority_class='session_or_organization_member' AND EXISTS (
    SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
      AND actor.status='active' AND actor.deleted_at_ms IS NULL
      AND (receipt.legacy_tenant_id IS NULL OR EXISTS (
        SELECT 1 FROM organizations organization
        LEFT JOIN organization_members member
          ON member.organization_id=organization.id AND member.user_id=actor.id
            AND member.state='active'
        WHERE organization.id=receipt.tenant_id AND organization.status='active'
          AND (organization.owner_id=actor.id OR member.user_id IS NOT NULL)))))
  OR (receipt.authority_class IN ('session_or_organization_owner','signed_state_or_organization_owner')
    AND EXISTS (SELECT 1 FROM users actor WHERE actor.id=receipt.actor_id
      AND actor.status='active' AND actor.deleted_at_ms IS NULL
      AND (receipt.legacy_tenant_id IS NULL OR EXISTS (
        SELECT 1 FROM organizations organization WHERE organization.id=receipt.tenant_id
          AND organization.status='active' AND organization.owner_id=actor.id))))
  OR (receipt.authority_class='organization_member' AND (
    (receipt.source_operation_id='cap-v1-60f863b2cb19353f'
      AND receipt.target_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM videos video WHERE video.id=receipt.target_id
          AND video.owner_id=receipt.actor_id
          AND video.organization_id=receipt.tenant_id
          AND video.deleted_at_ms IS NULL))
    OR ((receipt.source_operation_id<>'cap-v1-60f863b2cb19353f'
        OR receipt.target_id IS NULL) AND EXISTS (
      SELECT 1 FROM organizations organization
      LEFT JOIN organization_members member
        ON member.organization_id=organization.id AND member.user_id=receipt.actor_id
          AND member.state='active'
      WHERE organization.id=receipt.tenant_id AND organization.status='active'
        AND (organization.owner_id=receipt.actor_id OR member.user_id IS NOT NULL)))))
  OR (receipt.authority_class='organization_manager' AND EXISTS (
    SELECT 1 FROM organizations organization
    LEFT JOIN organization_members member
      ON member.organization_id=organization.id AND member.user_id=receipt.actor_id
        AND member.state='active'
    WHERE organization.id=receipt.tenant_id AND organization.status='active'
      AND (organization.owner_id=receipt.actor_id OR member.role IN ('owner','admin'))))
  OR (receipt.authority_class='organization_owner' AND EXISTS (
    SELECT 1 FROM organizations organization WHERE organization.id=receipt.tenant_id
      AND organization.status='active' AND organization.owner_id=receipt.actor_id))
  OR (receipt.authority_class='space_manager' AND EXISTS (
    SELECT 1 FROM spaces space JOIN organizations organization
      ON organization.id=space.organization_id
    LEFT JOIN organization_members member
      ON member.organization_id=organization.id AND member.user_id=receipt.actor_id
        AND member.state='active'
    LEFT JOIN space_members space_member
      ON space_member.space_id=space.id AND space_member.user_id=receipt.actor_id
    WHERE space.id=receipt.target_id AND space.deleted_at_ms IS NULL
      AND organization.status='active'
      AND (organization.owner_id=receipt.actor_id OR member.role IN ('owner','admin')
        OR space_member.role='manager')))
  OR (receipt.authority_class='video_viewer' AND EXISTS (
    SELECT 1 FROM legacy_protected_integration_video_policy_live_v1 policy
    WHERE policy.receipt_id=receipt.receipt_id))
  OR (receipt.authority_class='parent_receipt'
    AND receipt.source_operation_id IN (
      'cap-v1-b9fcb0fbd25b2234','cap-v1-bd1b9d67380624f7'
    ));

-- The early runtime read improves error classification; this AFTER trigger is
-- the transaction fence and rolls the whole D1 batch back if authority changed.
CREATE TRIGGER legacy_protected_integration_receipt_authority_gate_v1
AFTER INSERT ON legacy_protected_integration_receipts_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_live_authority_v1 live
  WHERE live.receipt_id=NEW.receipt_id AND live.authority_expires_at_ms>NEW.created_at_ms
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_authority_stale_v1'); END;

-- Only source-observed workflow launches are admitted. A child-derived edge
-- permits a parent with no final video id to launch a workflow whose target is
-- resolved by the child payload; same-target edges require exact identity.
INSERT OR IGNORE INTO legacy_protected_effect_parent_edges_v1 (
  parent_family,parent_operation_id,child_family,child_operation_id,
  target_binding_rule
) VALUES
  ('protected_integrations','cap-v1-cb3fade2af06d6bd',
    'protected_integrations','cap-v1-b9fcb0fbd25b2234','child_derived'),
  ('protected_integrations','cap-v1-d062d262b013a0cd',
    'protected_integrations','cap-v1-b9fcb0fbd25b2234','child_derived'),
  ('protected_integrations','cap-v1-f0a00e93ab606a52',
    'protected_integrations','cap-v1-bd1b9d67380624f7','same');

CREATE TRIGGER legacy_protected_integration_workflow_parent_gate_v1
AFTER INSERT ON legacy_protected_integration_receipts_v1
WHEN NEW.operation_kind='workflow' AND NOT EXISTS (
  SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
  JOIN legacy_protected_effect_parent_edges_v1 edge
    ON edge.parent_family=parent.parent_family
   AND edge.parent_operation_id=parent.source_operation_id
   AND edge.child_family='protected_integrations'
   AND edge.child_operation_id=NEW.source_operation_id
  WHERE parent.parent_family=NEW.parent_family
    AND parent.parent_receipt_id=NEW.parent_receipt_id
    AND parent.request_digest=NEW.parent_request_digest
    AND parent.authority_binding_digest=NEW.parent_authority_binding_digest
    AND parent.state<>'dead_letter'
    AND parent.created_at_ms<=NEW.created_at_ms
    AND parent.actor_id IS NEW.actor_id
    AND parent.tenant_id IS NEW.tenant_id
    AND parent.credential_kind=NEW.credential_kind
    AND parent.credential_subject_id IS NEW.credential_subject_id
    AND parent.credential_key_version IS NEW.credential_key_version
    AND parent.credential_digest IS NEW.credential_digest
    AND parent.credential_expires_at_ms IS NEW.credential_expires_at_ms
    AND parent.policy_proofs_json=NEW.policy_proofs_json
    AND parent.entitlement_kind IS NEW.entitlement_kind
    AND parent.entitlement_subject_id IS NEW.entitlement_subject_id
    AND parent.entitlement_revision IS NEW.entitlement_revision
    AND parent.entitlement_expires_at_ms IS NEW.entitlement_expires_at_ms
    AND (
      (edge.target_binding_rule='same' AND parent.target_id IS NEW.target_id)
      OR (edge.target_binding_rule='child_derived' AND NEW.target_id IS NOT NULL)
    )
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_workflow_parent_invalid_v1'); END;

-- Every admitted integration receipt is mirrored into the neutral registry so
-- later integration or media workflows can inherit the exact immutable proof.
CREATE TRIGGER legacy_protected_integration_parent_registry_insert_v1
AFTER INSERT ON legacy_protected_integration_receipts_v1
BEGIN
  INSERT INTO legacy_protected_effect_parent_registry_v1(
    parent_family,parent_receipt_id,source_operation_id,request_digest,
    actor_id,tenant_id,target_id,auth_class,authority_class,
    credential_kind,credential_subject_id,credential_key_version,credential_digest,
    credential_expires_at_ms,
    policy_proofs_json,entitlement_kind,entitlement_subject_id,
    entitlement_revision,entitlement_expires_at_ms,authority_binding_digest,
    state,created_at_ms,completed_at_ms
  ) VALUES(
    'protected_integrations',NEW.receipt_id,NEW.source_operation_id,NEW.request_digest,
    NEW.actor_id,NEW.tenant_id,NEW.target_id,NEW.auth_class,NEW.authority_class,
    NEW.credential_kind,NEW.credential_subject_id,NEW.credential_key_version,
    NEW.credential_digest,NEW.credential_expires_at_ms,NEW.policy_proofs_json,
    NEW.entitlement_kind,
    NEW.entitlement_subject_id,NEW.entitlement_revision,NEW.entitlement_expires_at_ms,
    NEW.authority_binding_digest,NEW.state,NEW.created_at_ms,NEW.completed_at_ms
  );
END;

CREATE TRIGGER legacy_protected_integration_parent_registry_state_v1
AFTER UPDATE OF state ON legacy_protected_integration_receipts_v1
BEGIN
  UPDATE legacy_protected_effect_parent_registry_v1
  SET state=NEW.state,completed_at_ms=NEW.completed_at_ms
  WHERE parent_family='protected_integrations'
    AND parent_receipt_id=NEW.receipt_id;
END;

-- Executors are provisioned by an operational identity outside request
-- handling. A short-lived lease binds one executor to the exact immutable
-- receipt, outbox payload and authority snapshot it is permitted to execute.
CREATE TABLE legacy_protected_integration_executors_v1 (
  executor_id TEXT PRIMARY KEY NOT NULL CHECK (length(executor_id) BETWEEN 1 AND 255),
  provider_kind TEXT NOT NULL CHECK (length(provider_kind) BETWEEN 1 AND 96),
  identity_digest TEXT NOT NULL CHECK (
    length(identity_digest)=64 AND identity_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('active','disabled'))
);

CREATE TABLE legacy_protected_integration_executor_leases_v1 (
  lease_id TEXT PRIMARY KEY NOT NULL CHECK (length(lease_id)=36),
  receipt_id TEXT NOT NULL
    REFERENCES legacy_protected_integration_receipts_v1(receipt_id) ON DELETE RESTRICT,
  executor_id TEXT NOT NULL
    REFERENCES legacy_protected_integration_executors_v1(executor_id) ON DELETE RESTRICT,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest)=64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outbox_payload_digest TEXT NOT NULL CHECK (
    length(outbox_payload_digest)=64
    AND outbox_payload_digest NOT GLOB '*[^0-9a-f]*'
  ),
  authority_binding_digest TEXT NOT NULL CHECK (
    length(authority_binding_digest)=64
    AND authority_binding_digest NOT GLOB '*[^0-9a-f]*'
  ),
  leased_at_ms INTEGER NOT NULL CHECK (
    leased_at_ms BETWEEN 0 AND 9007199254740991
  ),
  lease_expires_at_ms INTEGER NOT NULL CHECK (
    lease_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  state TEXT NOT NULL CHECK (state IN ('active','consumed','expired')),
  CHECK (lease_expires_at_ms>leased_at_ms
    AND lease_expires_at_ms<=leased_at_ms+900000),
  UNIQUE(receipt_id,lease_id)
);
CREATE UNIQUE INDEX legacy_protected_integration_one_active_lease_v1
  ON legacy_protected_integration_executor_leases_v1(receipt_id)
  WHERE state='active';

CREATE TRIGGER legacy_protected_integration_lease_insert_gate_v1
BEFORE INSERT ON legacy_protected_integration_executor_leases_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_receipts_v1 receipt
  JOIN legacy_protected_integration_outbox_v1 outbox
    ON outbox.receipt_id=receipt.receipt_id
  JOIN legacy_protected_integration_live_authority_v1 live
    ON live.receipt_id=receipt.receipt_id
  JOIN legacy_protected_integration_executors_v1 executor
    ON executor.executor_id=NEW.executor_id
  WHERE receipt.receipt_id=NEW.receipt_id
    AND receipt.request_digest=NEW.request_digest
    AND receipt.authority_binding_digest=NEW.authority_binding_digest
    AND outbox.payload_digest=NEW.outbox_payload_digest
    AND receipt.provider_kind=executor.provider_kind
    AND outbox.provider_kind=executor.provider_kind
    AND receipt.state='pending_provider_evidence'
    AND outbox.state='pending_provider_evidence'
    AND executor.state='active'
    AND NEW.leased_at_ms>=receipt.created_at_ms
    AND live.authority_expires_at_ms>NEW.leased_at_ms
    AND (receipt.operation_kind<>'workflow' OR EXISTS (
      SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
      JOIN legacy_protected_effect_parent_edges_v1 edge
        ON edge.parent_family=parent.parent_family
       AND edge.parent_operation_id=parent.source_operation_id
       AND edge.child_family='protected_integrations'
       AND edge.child_operation_id=receipt.source_operation_id
      WHERE parent.parent_family=receipt.parent_family
        AND parent.parent_receipt_id=receipt.parent_receipt_id
        AND parent.request_digest=receipt.parent_request_digest
        AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
        AND parent.state<>'dead_letter'
        AND parent.actor_id IS receipt.actor_id
        AND parent.tenant_id IS receipt.tenant_id
        AND parent.credential_kind=receipt.credential_kind
        AND parent.credential_subject_id IS receipt.credential_subject_id
        AND parent.credential_key_version IS receipt.credential_key_version
        AND parent.credential_digest IS receipt.credential_digest
        AND parent.credential_expires_at_ms IS receipt.credential_expires_at_ms
        AND parent.policy_proofs_json=receipt.policy_proofs_json
        AND parent.entitlement_kind IS receipt.entitlement_kind
        AND parent.entitlement_subject_id IS receipt.entitlement_subject_id
        AND parent.entitlement_revision IS receipt.entitlement_revision
        AND parent.entitlement_expires_at_ms IS receipt.entitlement_expires_at_ms
        AND (
          (edge.target_binding_rule='same' AND parent.target_id IS receipt.target_id)
          OR (edge.target_binding_rule='child_derived' AND receipt.target_id IS NOT NULL)
        )
    ))
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_lease_invalid_v1'); END;

CREATE TABLE legacy_protected_integration_evidence_v1 (
  receipt_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_protected_integration_receipts_v1(receipt_id) ON DELETE RESTRICT,
  lease_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_protected_integration_executor_leases_v1(lease_id) ON DELETE RESTRICT,
  executor_id TEXT NOT NULL
    REFERENCES legacy_protected_integration_executors_v1(executor_id) ON DELETE RESTRICT,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest)=64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outbox_payload_digest TEXT NOT NULL CHECK (
    length(outbox_payload_digest)=64
    AND outbox_payload_digest NOT GLOB '*[^0-9a-f]*'
  ),
  authority_binding_digest TEXT NOT NULL CHECK (
    length(authority_binding_digest)=64
    AND authority_binding_digest NOT GLOB '*[^0-9a-f]*'
  ),
  provider_evidence_digest TEXT NOT NULL CHECK (
    length(provider_evidence_digest)=64
    AND provider_evidence_digest NOT GLOB '*[^0-9a-f]*'
  ),
  terminal_kind TEXT NOT NULL CHECK (terminal_kind IN ('http','json','workflow')),
  sealed_terminal_ref TEXT NOT NULL CHECK (
    length(sealed_terminal_ref)=85
    AND substr(sealed_terminal_ref,1,21)='frame-pi-terminal-v1:'
    AND substr(sealed_terminal_ref,22) NOT GLOB '*[^0-9a-f]*'
  ),
  sealed_terminal_digest TEXT NOT NULL CHECK (
    length(sealed_terminal_digest)=64
    AND sealed_terminal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  terminal_expires_at_ms INTEGER NOT NULL CHECK (
    terminal_expires_at_ms BETWEEN 1 AND 9007199254740991
  ),
  verified_at_ms INTEGER NOT NULL CHECK (
    verified_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (terminal_expires_at_ms>verified_at_ms
    AND terminal_expires_at_ms<=verified_at_ms+900000)
);

CREATE TRIGGER legacy_protected_integration_evidence_gate_v1
BEFORE INSERT ON legacy_protected_integration_evidence_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_receipts_v1 receipt
  JOIN legacy_protected_integration_outbox_v1 outbox
    ON outbox.receipt_id=receipt.receipt_id
  JOIN legacy_protected_integration_live_authority_v1 live
    ON live.receipt_id=receipt.receipt_id
  JOIN legacy_protected_integration_executor_leases_v1 lease
    ON lease.receipt_id=receipt.receipt_id
  JOIN legacy_protected_integration_executors_v1 executor
    ON executor.executor_id=lease.executor_id
  WHERE receipt.receipt_id=NEW.receipt_id
    AND receipt.request_digest=NEW.request_digest
    AND receipt.authority_binding_digest=NEW.authority_binding_digest
    AND receipt.terminal_kind=NEW.terminal_kind
    AND outbox.payload_digest=NEW.outbox_payload_digest
    AND receipt.provider_kind=executor.provider_kind
    AND outbox.provider_kind=executor.provider_kind
    AND lease.lease_id=NEW.lease_id
    AND lease.executor_id=NEW.executor_id
    AND lease.request_digest=NEW.request_digest
    AND lease.outbox_payload_digest=NEW.outbox_payload_digest
    AND lease.authority_binding_digest=NEW.authority_binding_digest
    AND lease.state='active' AND executor.state='active'
    AND NEW.verified_at_ms>=lease.leased_at_ms
    AND NEW.verified_at_ms<lease.lease_expires_at_ms
    AND live.authority_expires_at_ms>NEW.verified_at_ms
    AND receipt.state='pending_provider_evidence'
    AND outbox.state='pending_provider_evidence'
    AND (receipt.operation_kind<>'workflow' OR EXISTS (
      SELECT 1 FROM legacy_protected_effect_parent_registry_v1 parent
      JOIN legacy_protected_effect_parent_edges_v1 edge
        ON edge.parent_family=parent.parent_family
       AND edge.parent_operation_id=parent.source_operation_id
       AND edge.child_family='protected_integrations'
       AND edge.child_operation_id=receipt.source_operation_id
      WHERE parent.parent_family=receipt.parent_family
        AND parent.parent_receipt_id=receipt.parent_receipt_id
        AND parent.request_digest=receipt.parent_request_digest
        AND parent.authority_binding_digest=receipt.parent_authority_binding_digest
        AND parent.state<>'dead_letter'
        AND parent.actor_id IS receipt.actor_id
        AND parent.tenant_id IS receipt.tenant_id
        AND parent.credential_kind=receipt.credential_kind
        AND parent.credential_subject_id IS receipt.credential_subject_id
        AND parent.credential_key_version IS receipt.credential_key_version
        AND parent.credential_digest IS receipt.credential_digest
        AND parent.credential_expires_at_ms IS receipt.credential_expires_at_ms
        AND parent.policy_proofs_json=receipt.policy_proofs_json
        AND parent.entitlement_kind IS receipt.entitlement_kind
        AND parent.entitlement_subject_id IS receipt.entitlement_subject_id
        AND parent.entitlement_revision IS receipt.entitlement_revision
        AND parent.entitlement_expires_at_ms IS receipt.entitlement_expires_at_ms
        AND (
          (edge.target_binding_rule='same' AND parent.target_id IS receipt.target_id)
          OR (edge.target_binding_rule='child_derived' AND receipt.target_id IS NOT NULL)
        )
    ))
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_evidence_invalid_v1'); END;

CREATE TRIGGER legacy_protected_integration_outbox_verified_gate_v1
BEFORE UPDATE OF state ON legacy_protected_integration_outbox_v1
WHEN NEW.state='verified' AND NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_evidence_v1 evidence
  WHERE evidence.receipt_id=NEW.receipt_id
    AND evidence.outbox_payload_digest=NEW.payload_digest
    AND evidence.verified_at_ms=NEW.completed_at_ms
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_evidence_required_v1'); END;

CREATE TRIGGER legacy_protected_integration_receipt_verified_gate_v1
BEFORE UPDATE OF state ON legacy_protected_integration_receipts_v1
WHEN NEW.state='verified' AND NOT (
  EXISTS (
    SELECT 1 FROM legacy_protected_integration_evidence_v1 evidence
    WHERE evidence.receipt_id=NEW.receipt_id
      AND evidence.request_digest=NEW.request_digest
      AND evidence.authority_binding_digest=NEW.authority_binding_digest
      AND evidence.terminal_kind=NEW.terminal_kind
      AND evidence.verified_at_ms=NEW.completed_at_ms
  )
  AND EXISTS (
    SELECT 1 FROM legacy_protected_integration_outbox_v1 outbox
    WHERE outbox.receipt_id=NEW.receipt_id AND outbox.state='verified'
      AND outbox.completed_at_ms=NEW.completed_at_ms
  )
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_evidence_required_v1'); END;

-- Evidence admission and all terminal state transitions occur in one SQLite
-- statement transaction. A verified receipt can therefore never be observed
-- with an active lease or pending outbox.
CREATE TRIGGER legacy_protected_integration_evidence_finalize_v1
AFTER INSERT ON legacy_protected_integration_evidence_v1
BEGIN
  UPDATE legacy_protected_integration_executor_leases_v1
  SET state='consumed'
  WHERE lease_id=NEW.lease_id AND receipt_id=NEW.receipt_id AND state='active';
  UPDATE legacy_protected_integration_outbox_v1
  SET state='verified',completed_at_ms=NEW.verified_at_ms
  WHERE receipt_id=NEW.receipt_id AND state='pending_provider_evidence';
  UPDATE legacy_protected_integration_receipts_v1
  SET state='verified',completed_at_ms=NEW.verified_at_ms
  WHERE receipt_id=NEW.receipt_id AND state='pending_provider_evidence';
END;

CREATE TRIGGER legacy_protected_integration_receipt_dead_letter_gate_v1
BEFORE UPDATE OF state ON legacy_protected_integration_receipts_v1
WHEN NEW.state='dead_letter' AND NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_outbox_v1 outbox
  WHERE outbox.receipt_id=NEW.receipt_id AND outbox.state='dead_letter'
    AND outbox.completed_at_ms=NEW.completed_at_ms
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_dead_letter_invalid_v1'); END;

CREATE TRIGGER legacy_protected_integration_receipt_immutable_v1
BEFORE UPDATE ON legacy_protected_integration_receipts_v1
WHEN NOT (
  OLD.receipt_id=NEW.receipt_id
  AND OLD.source_operation_id=NEW.source_operation_id
  AND OLD.operation_kind=NEW.operation_kind AND OLD.method=NEW.method
  AND OLD.surface_path=NEW.surface_path AND OLD.auth_class=NEW.auth_class
  AND OLD.authority_class=NEW.authority_class AND OLD.provider_kind=NEW.provider_kind
  AND OLD.principal_digest=NEW.principal_digest
  AND OLD.actor_id IS NEW.actor_id AND OLD.tenant_id IS NEW.tenant_id
  AND OLD.target_id IS NEW.target_id AND OLD.tenant_domain=NEW.tenant_domain
  AND OLD.target_domain=NEW.target_domain
  AND OLD.legacy_tenant_id IS NEW.legacy_tenant_id
  AND OLD.legacy_target_id IS NEW.legacy_target_id
  AND OLD.legacy_workflow_actor_id IS NEW.legacy_workflow_actor_id
  AND OLD.legacy_workflow_cap_tenant_id IS NEW.legacy_workflow_cap_tenant_id
  AND OLD.workflow_raw_file_key IS NEW.workflow_raw_file_key
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
  AND OLD.conditional_bindings_json=NEW.conditional_bindings_json
  AND OLD.authority_binding_digest=NEW.authority_binding_digest
  AND OLD.parent_family IS NEW.parent_family
  AND OLD.parent_receipt_id IS NEW.parent_receipt_id
  AND OLD.parent_request_digest IS NEW.parent_request_digest
  AND OLD.parent_authority_binding_digest IS NEW.parent_authority_binding_digest
  AND OLD.replay_key_digest=NEW.replay_key_digest
  AND OLD.replay_origin=NEW.replay_origin
  AND OLD.idempotency_mode=NEW.idempotency_mode
  AND OLD.request_digest=NEW.request_digest
  AND OLD.redacted_request_json=NEW.redacted_request_json
  AND OLD.sealed_request_ref=NEW.sealed_request_ref
  AND OLD.sealed_request_digest=NEW.sealed_request_digest
  AND OLD.transport_body_digest IS NEW.transport_body_digest
  AND OLD.terminal_kind=NEW.terminal_kind
  AND OLD.conditional_duration_seconds IS NEW.conditional_duration_seconds
  AND OLD.conditional_password_requested=NEW.conditional_password_requested
  AND OLD.conditional_pro_settings_requested=NEW.conditional_pro_settings_requested
  AND OLD.conditional_public_requested=NEW.conditional_public_requested
  AND OLD.seat_quantity IS NEW.seat_quantity
  AND OLD.created_at_ms=NEW.created_at_ms
  AND OLD.state='pending_provider_evidence'
  AND NEW.state IN ('verified','dead_letter')
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_receipt_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_receipt_no_delete_v1
BEFORE DELETE ON legacy_protected_integration_receipts_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_receipt_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_outbox_update_gate_v1
BEFORE UPDATE ON legacy_protected_integration_outbox_v1
WHEN NOT (
  OLD.receipt_id=NEW.receipt_id AND OLD.provider_kind=NEW.provider_kind
  AND OLD.payload_json=NEW.payload_json AND OLD.payload_digest=NEW.payload_digest
  AND OLD.created_at_ms=NEW.created_at_ms
  AND OLD.state='pending_provider_evidence'
  AND NEW.state IN ('pending_provider_evidence','verified','dead_letter')
  AND NEW.attempt_count BETWEEN OLD.attempt_count AND 1000000
  AND (
    (NEW.state='pending_provider_evidence' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state IN ('verified','dead_letter') AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_outbox_no_delete_v1
BEFORE DELETE ON legacy_protected_integration_outbox_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_executor_update_gate_v1
BEFORE UPDATE ON legacy_protected_integration_executors_v1
WHEN NOT (
  OLD.executor_id=NEW.executor_id AND OLD.provider_kind=NEW.provider_kind
  AND OLD.identity_digest=NEW.identity_digest
  AND OLD.state='active' AND NEW.state='disabled'
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_executor_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_executor_no_delete_v1
BEFORE DELETE ON legacy_protected_integration_executors_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_executor_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_lease_update_gate_v1
BEFORE UPDATE ON legacy_protected_integration_executor_leases_v1
WHEN NOT (
  OLD.lease_id=NEW.lease_id AND OLD.receipt_id=NEW.receipt_id
  AND OLD.executor_id=NEW.executor_id AND OLD.request_digest=NEW.request_digest
  AND OLD.outbox_payload_digest=NEW.outbox_payload_digest
  AND OLD.authority_binding_digest=NEW.authority_binding_digest
  AND OLD.leased_at_ms=NEW.leased_at_ms
  AND OLD.lease_expires_at_ms=NEW.lease_expires_at_ms
  AND OLD.state='active' AND NEW.state IN ('consumed','expired')
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_lease_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_lease_no_delete_v1
BEFORE DELETE ON legacy_protected_integration_executor_leases_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_lease_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_evidence_immutable_v1
BEFORE UPDATE ON legacy_protected_integration_evidence_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_evidence_immutable_v1'); END;

CREATE TRIGGER legacy_protected_integration_evidence_no_delete_v1
BEFORE DELETE ON legacy_protected_integration_evidence_v1
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_evidence_immutable_v1'); END;

-- Persisted request JSON is an exact digest-only projection. Unknown fields
-- are rejected so a newly introduced provider secret cannot silently become a
-- D1 column or JSON value before the application redactor learns its name.
CREATE TRIGGER legacy_protected_integration_request_redaction_gate_v1
BEFORE INSERT ON legacy_protected_integration_receipts_v1
WHEN NOT (
  json_extract(NEW.redacted_request_json,'$.schema_version')=
    'frame.legacy-protected-integration-request.v1'
  AND json_extract(NEW.redacted_request_json,'$.source_operation_id')=
    NEW.source_operation_id
  AND json_extract(NEW.redacted_request_json,'$.payload.digest_only')=1
  AND length(json_extract(NEW.redacted_request_json,'$.payload.sha256'))=64
  AND json_extract(NEW.redacted_request_json,'$.payload.sha256')
    NOT GLOB '*[^0-9a-f]*'
  AND json_extract(NEW.redacted_request_json,'$.sealed_request_digest')=
    NEW.sealed_request_digest
  AND json_extract(NEW.redacted_request_json,'$.transport_body_digest')
    IS NEW.transport_body_digest
  AND json_extract(NEW.redacted_request_json,'$.parent_family') IS NEW.parent_family
  AND json_extract(NEW.redacted_request_json,'$.parent_receipt_id')
    IS NEW.parent_receipt_id
  AND json_extract(NEW.redacted_request_json,'$.parent_request_digest')
    IS NEW.parent_request_digest
  AND json_extract(NEW.redacted_request_json,'$.parent_authority_binding_digest')
    IS NEW.parent_authority_binding_digest
  AND (SELECT COUNT(*) FROM json_each(NEW.redacted_request_json))=9
  AND (SELECT COUNT(*) FROM json_each(
    json_extract(NEW.redacted_request_json,'$.payload')))=2
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_request_not_redacted_v1'); END;

-- The outbox carries only the exact redacted receipt descriptor and opaque
-- vault reference. Provider inputs and terminal bytes are never embedded.
CREATE TRIGGER legacy_protected_integration_outbox_payload_gate_v1
BEFORE INSERT ON legacy_protected_integration_outbox_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_protected_integration_receipts_v1 receipt
  WHERE receipt.receipt_id=NEW.receipt_id
    AND NEW.provider_kind=receipt.provider_kind
    AND json_extract(NEW.payload_json,'$.schema_version')=
      'frame.legacy-protected-integration-outbox.v1'
    AND json_extract(NEW.payload_json,'$.receipt_id')=receipt.receipt_id
    AND json_extract(NEW.payload_json,'$.source_operation_id')=receipt.source_operation_id
    AND json_extract(NEW.payload_json,'$.provider')=receipt.provider_kind
    AND json_extract(NEW.payload_json,'$.principal_digest')=receipt.principal_digest
    AND json_extract(NEW.payload_json,'$.tenant_id') IS receipt.tenant_id
    AND json_extract(NEW.payload_json,'$.target_id') IS receipt.target_id
    AND json_extract(NEW.payload_json,'$.legacy_tenant_id') IS receipt.legacy_tenant_id
    AND json_extract(NEW.payload_json,'$.legacy_target_id') IS receipt.legacy_target_id
    AND json_extract(NEW.payload_json,'$.request_digest')=receipt.request_digest
    AND json_extract(NEW.payload_json,'$.authority_binding_digest')=
      receipt.authority_binding_digest
    AND json_extract(NEW.payload_json,'$.sealed_request_ref')=receipt.sealed_request_ref
    AND json_extract(NEW.payload_json,'$.sealed_request_digest')=
      receipt.sealed_request_digest
    AND json_extract(NEW.payload_json,'$.release_gate')=
      'independent_provider_executor_evidence'
    AND json_extract(NEW.payload_json,'$.redacted_request.schema_version')=
      'frame.legacy-protected-integration-request.v1'
    AND json_extract(NEW.payload_json,'$.redacted_request.payload.digest_only')=1
    AND (SELECT COUNT(*) FROM json_each(NEW.payload_json))=19
)
BEGIN SELECT RAISE(ABORT,'frame_protected_integration_outbox_payload_invalid_v1'); END;
