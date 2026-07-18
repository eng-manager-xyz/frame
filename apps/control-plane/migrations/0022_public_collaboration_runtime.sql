PRAGMA foreign_keys = ON;

-- Public share capabilities are opaque, short-lived, and scoped to one
-- already-public video. Only their SHA-256 digests are persisted.
CREATE TABLE public_collaboration_policies_v1 (
  organization_id TEXT PRIMARY KEY NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  anonymous_comments_enabled INTEGER NOT NULL CHECK (anonymous_comments_enabled IN (0, 1)),
  comment_moderation TEXT NOT NULL CHECK (comment_moderation IN ('publish', 'pre_moderate')),
  comment_maximum_per_minute INTEGER NOT NULL CHECK (comment_maximum_per_minute BETWEEN 1 AND 60),
  analytics_enabled INTEGER NOT NULL CHECK (analytics_enabled IN (0, 1)),
  analytics_consent_required INTEGER NOT NULL DEFAULT 1 CHECK (analytics_consent_required = 1),
  analytics_policy_version TEXT NOT NULL
    CHECK (length(analytics_policy_version) BETWEEN 3 AND 64),
  analytics_retention_days INTEGER NOT NULL CHECK (analytics_retention_days BETWEEN 1 AND 90),
  analytics_maximum_per_minute INTEGER NOT NULL CHECK (analytics_maximum_per_minute BETWEEN 1 AND 120),
  revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991)
);

-- Grant issuance has a privacy-safe share bucket so rotating anonymous
-- capabilities cannot create unbounded state or reset all action ceilings.
CREATE TABLE public_collaboration_grant_rate_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  accepted_at_ms INTEGER NOT NULL CHECK (accepted_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > accepted_at_ms)
);
CREATE INDEX public_collaboration_grant_rate_v1_window_idx
  ON public_collaboration_grant_rate_v1(share_id, accepted_at_ms);
CREATE INDEX public_collaboration_grant_rate_v1_expiry_idx
  ON public_collaboration_grant_rate_v1(expires_at_ms, id);
CREATE TRIGGER public_collaboration_grant_rate_limit_v1
BEFORE INSERT ON public_collaboration_grant_rate_v1
WHEN (
  SELECT COUNT(*) FROM public_collaboration_grant_rate_v1 e
  WHERE e.share_id = NEW.share_id AND e.accepted_at_ms > NEW.accepted_at_ms - 60000
) >= 120
BEGIN
  SELECT RAISE(ABORT, 'frame_public_collaboration_grant_rate_limited_v1');
END;

-- Composite uniqueness exists solely so every denormalized tenant/share pair
-- below can be expressed as a real foreign key rather than trusted convention.
CREATE UNIQUE INDEX videos_id_organization_public_collaboration_v1_idx
  ON videos(id, organization_id);
CREATE UNIQUE INDEX comments_id_video_public_collaboration_v1_idx
  ON comments(id, video_id);

CREATE TABLE public_collaboration_grants_v1 (
  token_digest TEXT PRIMARY KEY NOT NULL
    CHECK (length(token_digest) = 64 AND token_digest NOT GLOB '*[^0-9a-f]*'),
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  comments_enabled INTEGER NOT NULL CHECK (comments_enabled IN (0, 1)),
  analytics_enabled INTEGER NOT NULL CHECK (analytics_enabled IN (0, 1)),
  analytics_policy_version TEXT NOT NULL
    CHECK (length(analytics_policy_version) BETWEEN 3 AND 64),
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > issued_at_ms),
  CHECK (revoked_at_ms IS NULL OR revoked_at_ms >= issued_at_ms),
  FOREIGN KEY (share_id, organization_id) REFERENCES videos(id, organization_id) ON DELETE CASCADE,
  UNIQUE (token_digest, share_id)
);
CREATE INDEX public_collaboration_grants_v1_expiry_idx
  ON public_collaboration_grants_v1(expires_at_ms, token_digest);
CREATE INDEX public_collaboration_grants_v1_share_idx
  ON public_collaboration_grants_v1(share_id, expires_at_ms);

CREATE TABLE public_comment_moderation_v1 (
  comment_id TEXT PRIMARY KEY NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK (state IN ('published', 'pending_moderation', 'rejected')),
  decided_at_ms INTEGER CHECK (decided_at_ms IS NULL OR decided_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  CHECK ((state = 'pending_moderation') = (decided_at_ms IS NULL)),
  FOREIGN KEY (comment_id, share_id) REFERENCES comments(id, video_id) ON DELETE CASCADE
);
CREATE INDEX public_comment_moderation_v1_share_state_idx
  ON public_comment_moderation_v1(share_id, state, comment_id);

CREATE TABLE public_comment_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  token_digest TEXT NOT NULL REFERENCES public_collaboration_grants_v1(token_digest) ON DELETE CASCADE,
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  payload_digest TEXT NOT NULL
    CHECK (length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
  comment_id TEXT NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
  response_json TEXT NOT NULL CHECK (json_valid(response_json) AND length(response_json) <= 8192),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms),
  FOREIGN KEY (token_digest, share_id)
    REFERENCES public_collaboration_grants_v1(token_digest, share_id) ON DELETE CASCADE,
  FOREIGN KEY (comment_id, share_id) REFERENCES comments(id, video_id) ON DELETE CASCADE
);
CREATE INDEX public_comment_operations_v1_expiry_idx
  ON public_comment_operations_v1(expires_at_ms, operation_id);

CREATE TABLE public_transcripts_v1 (
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  language TEXT NOT NULL CHECK (length(language) BETWEEN 2 AND 35),
  duration_ms INTEGER NOT NULL CHECK (duration_ms BETWEEN 0 AND 86400000),
  document_json TEXT NOT NULL
    CHECK (json_valid(document_json) AND length(document_json) <= 1250000),
  document_checksum TEXT NOT NULL
    CHECK (length(document_checksum) = 64 AND document_checksum NOT GLOB '*[^0-9a-f]*'),
  is_current INTEGER NOT NULL CHECK (is_current IN (0, 1)),
  published_at_ms INTEGER NOT NULL CHECK (published_at_ms BETWEEN 0 AND 9007199254740991),
  published_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  PRIMARY KEY (share_id, revision),
  FOREIGN KEY (share_id, organization_id) REFERENCES videos(id, organization_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX public_transcripts_v1_current_idx
  ON public_transcripts_v1(share_id) WHERE is_current = 1;

CREATE TABLE public_analytics_consents_v1 (
  token_digest TEXT NOT NULL REFERENCES public_collaboration_grants_v1(token_digest) ON DELETE CASCADE,
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  policy_version TEXT NOT NULL CHECK (length(policy_version) BETWEEN 3 AND 64),
  state TEXT NOT NULL CHECK (state IN ('granted', 'denied')),
  granted_at_ms INTEGER NOT NULL CHECK (granted_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (expires_at_ms > granted_at_ms),
  PRIMARY KEY (token_digest, share_id, policy_version),
  FOREIGN KEY (token_digest, share_id)
    REFERENCES public_collaboration_grants_v1(token_digest, share_id) ON DELETE CASCADE
);
CREATE INDEX public_analytics_consents_v1_expiry_idx
  ON public_analytics_consents_v1(expires_at_ms, token_digest);

CREATE TABLE public_analytics_consent_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  token_digest TEXT NOT NULL REFERENCES public_collaboration_grants_v1(token_digest) ON DELETE CASCADE,
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  payload_digest TEXT NOT NULL
    CHECK (length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
  response_json TEXT NOT NULL CHECK (json_valid(response_json) AND length(response_json) <= 4096),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms),
  FOREIGN KEY (token_digest, share_id)
    REFERENCES public_collaboration_grants_v1(token_digest, share_id) ON DELETE CASCADE
);
CREATE INDEX public_analytics_consent_operations_v1_expiry_idx
  ON public_analytics_consent_operations_v1(expires_at_ms, operation_id);

CREATE TABLE public_analytics_events_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  token_digest TEXT NOT NULL REFERENCES public_collaboration_grants_v1(token_digest) ON DELETE CASCADE,
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  policy_version TEXT NOT NULL CHECK (length(policy_version) BETWEEN 3 AND 64),
  payload_digest TEXT NOT NULL
    CHECK (length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
  sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 9007199254740991),
  kind TEXT NOT NULL CHECK (kind IN (
    'playback_started', 'playback_paused', 'playback_completed', 'playback_error'
  )),
  position_ms INTEGER CHECK (position_ms IS NULL OR position_ms BETWEEN 0 AND 86400000),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  recorded_at_ms INTEGER NOT NULL CHECK (recorded_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > recorded_at_ms),
  UNIQUE (token_digest, share_id, sequence),
  FOREIGN KEY (token_digest, share_id)
    REFERENCES public_collaboration_grants_v1(token_digest, share_id) ON DELETE CASCADE
);
CREATE INDEX public_analytics_events_v1_retention_idx
  ON public_analytics_events_v1(expires_at_ms, operation_id);
CREATE INDEX public_analytics_events_v1_share_time_idx
  ON public_analytics_events_v1(share_id, occurred_at_ms, operation_id);

CREATE TABLE public_collaboration_rate_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  token_digest TEXT NOT NULL REFERENCES public_collaboration_grants_v1(token_digest) ON DELETE CASCADE,
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  action TEXT NOT NULL CHECK (action IN ('comment', 'analytics')),
  accepted_at_ms INTEGER NOT NULL CHECK (accepted_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > accepted_at_ms),
  FOREIGN KEY (token_digest, share_id)
    REFERENCES public_collaboration_grants_v1(token_digest, share_id) ON DELETE CASCADE
);
CREATE INDEX public_collaboration_rate_events_v1_window_idx
  ON public_collaboration_rate_events_v1(token_digest, share_id, action, accepted_at_ms);
CREATE INDEX public_collaboration_rate_events_v1_expiry_idx
  ON public_collaboration_rate_events_v1(expires_at_ms, id);

CREATE TRIGGER public_collaboration_comment_rate_v1
BEFORE INSERT ON public_collaboration_rate_events_v1
WHEN NEW.action = 'comment' AND (
  (SELECT COUNT(*) FROM public_collaboration_rate_events_v1 e
   WHERE e.token_digest = NEW.token_digest AND e.share_id = NEW.share_id
     AND e.action = 'comment' AND e.accepted_at_ms > NEW.accepted_at_ms - 60000)
    >= COALESCE((
      SELECT p.comment_maximum_per_minute
      FROM videos v JOIN public_collaboration_policies_v1 p
        ON p.organization_id = v.organization_id WHERE v.id = NEW.share_id
    ), 0)
  OR
  (SELECT COUNT(*) FROM public_collaboration_rate_events_v1 e
   WHERE e.share_id = NEW.share_id AND e.action = 'comment'
     AND e.accepted_at_ms > NEW.accepted_at_ms - 60000)
    >= COALESCE((
      SELECT p.comment_maximum_per_minute
      FROM videos v JOIN public_collaboration_policies_v1 p
        ON p.organization_id = v.organization_id WHERE v.id = NEW.share_id
    ), 0)
)
BEGIN
  SELECT RAISE(ABORT, 'frame_public_collaboration_rate_limited_v1');
END;

CREATE TRIGGER public_collaboration_analytics_rate_v1
BEFORE INSERT ON public_collaboration_rate_events_v1
WHEN NEW.action = 'analytics' AND (
  (SELECT COUNT(*) FROM public_collaboration_rate_events_v1 e
   WHERE e.token_digest = NEW.token_digest AND e.share_id = NEW.share_id
     AND e.action = 'analytics' AND e.accepted_at_ms > NEW.accepted_at_ms - 60000)
    >= COALESCE((
      SELECT p.analytics_maximum_per_minute
      FROM videos v JOIN public_collaboration_policies_v1 p
        ON p.organization_id = v.organization_id WHERE v.id = NEW.share_id
    ), 0)
  OR
  (SELECT COUNT(*) FROM public_collaboration_rate_events_v1 e
   WHERE e.share_id = NEW.share_id AND e.action = 'analytics'
     AND e.accepted_at_ms > NEW.accepted_at_ms - 60000)
    >= COALESCE((
      SELECT p.analytics_maximum_per_minute
      FROM videos v JOIN public_collaboration_policies_v1 p
        ON p.organization_id = v.organization_id WHERE v.id = NEW.share_id
    ), 0)
)
BEGIN
  SELECT RAISE(ABORT, 'frame_public_collaboration_rate_limited_v1');
END;

CREATE TABLE public_collaboration_audit_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  share_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  token_digest TEXT NOT NULL
    CHECK (length(token_digest) = 64 AND token_digest NOT GLOB '*[^0-9a-f]*'),
  action TEXT NOT NULL CHECK (action IN (
    'grant_issued', 'comment_created', 'transcript_published',
    'analytics_consent', 'analytics_recorded', 'retention_pruned'
  )),
  outcome TEXT NOT NULL CHECK (outcome IN ('applied', 'duplicate', 'ignored')),
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) BETWEEN 1 AND 128),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX public_collaboration_audit_v1_share_time_idx
  ON public_collaboration_audit_v1(share_id, occurred_at_ms, id);

CREATE TRIGGER public_collaboration_audit_v1_immutable_update
BEFORE UPDATE ON public_collaboration_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'public collaboration audit is immutable');
END;
CREATE TRIGGER public_collaboration_audit_v1_immutable_delete
BEFORE DELETE ON public_collaboration_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'public collaboration audit is immutable');
END;
