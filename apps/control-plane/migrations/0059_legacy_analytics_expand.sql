PRAGMA foreign_keys = ON;

-- Cap marks the first-view email before attempting delivery. Keeping that
-- claim in D1 prevents retries or concurrent views from producing duplicates.
ALTER TABLE videos ADD COLUMN legacy_analytics_first_view_email_sent_at_ms INTEGER
  CHECK (
    legacy_analytics_first_view_email_sent_at_ms IS NULL
    OR legacy_analytics_first_view_email_sent_at_ms BETWEEN 0 AND 9007199254740991
  );

-- Password verification is represented as an expiring bearer grant. The
-- password verifier/cookie executor may mint this row; analytics only consumes
-- it and never trusts a caller-provided "authorized" boolean.
CREATE TABLE legacy_analytics_password_grants_v1 (
  grant_digest TEXT NOT NULL CHECK (
    length(grant_digest) = 64 AND grant_digest NOT GLOB '*[^0-9a-f]*'
  ),
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (grant_digest, video_id),
  CHECK (expires_at_ms > issued_at_ms)
);
CREATE INDEX legacy_analytics_password_grants_expiry_v1
  ON legacy_analytics_password_grants_v1(expires_at_ms, video_id);

-- Every provider-bound request is claimed before an external query/event.
-- Reads receive a fresh server execution key; mutation callers may supply an
-- idempotency key, but the released analytics beacon does not send one.
CREATE TABLE legacy_analytics_provider_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-c8a43dc80c502b6d', 'cap-v1-51dc2aa9f19a48cc',
    'cap-v1-9b093898957efebb', 'cap-v1-be2ea6b474aae7c9',
    'cap-v1-7c47f9a2a9a24ac0', 'cap-v1-9186738740a1ece1'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN ('query','event')),
  principal_digest TEXT NOT NULL CHECK (
    length(principal_digest) = 64 AND principal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  actor_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  active_organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  target_video_id TEXT CHECK (
    target_video_id IS NULL OR length(target_video_id) BETWEEN 1 AND 255
  ),
  execution_key_digest TEXT NOT NULL CHECK (
    length(execution_key_digest) = 64 AND execution_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending','complete','dead_letter')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  UNIQUE (source_operation_id, principal_digest, execution_key_digest),
  CHECK (
    (source_operation_id = 'cap-v1-51dc2aa9f19a48cc' AND operation_kind = 'event')
    OR (source_operation_id <> 'cap-v1-51dc2aa9f19a48cc' AND operation_kind = 'query')
  ),
  CHECK (
    (state = 'pending' AND completed_at_ms IS NULL)
    OR (state IN ('complete','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_analytics_query_outbox_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_analytics_provider_operations_v1(operation_id) ON DELETE RESTRICT,
  query_kind TEXT NOT NULL CHECK (query_kind IN (
    'video_count_route','dashboard','video_count_http',
    'video_count_bulk','video_count_action'
  )),
  request_json TEXT NOT NULL CHECK (
    json_valid(request_json) AND json_type(request_json) = 'object'
    AND length(request_json) <= 262144
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','complete','dead_letter')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'pending' AND completed_at_ms IS NULL)
    OR (state IN ('complete','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_analytics_event_outbox_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_analytics_provider_operations_v1(operation_id) ON DELETE RESTRICT,
  timestamp_iso TEXT NOT NULL CHECK (length(timestamp_iso) BETWEEN 20 AND 64),
  session_id TEXT NOT NULL CHECK (length(session_id) BETWEEN 1 AND 128),
  action TEXT NOT NULL DEFAULT 'page_hit' CHECK (action = 'page_hit'),
  version TEXT NOT NULL DEFAULT '1.0' CHECK (version = '1.0'),
  tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 256),
  video_id TEXT NOT NULL CHECK (length(video_id) BETWEEN 1 AND 255),
  pathname TEXT NOT NULL CHECK (length(pathname) <= 8192),
  country TEXT NOT NULL CHECK (length(country) <= 256),
  region TEXT NOT NULL CHECK (length(region) <= 256),
  city TEXT NOT NULL CHECK (length(city) <= 256),
  browser TEXT NOT NULL CHECK (length(browser) BETWEEN 1 AND 256),
  device TEXT NOT NULL CHECK (length(device) BETWEEN 1 AND 256),
  operating_system TEXT NOT NULL CHECK (length(operating_system) BETWEEN 1 AND 256),
  raw_user_agent TEXT NOT NULL CHECK (length(raw_user_agent) BETWEEN 1 AND 1024),
  user_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  event_json TEXT NOT NULL CHECK (
    json_valid(event_json) AND json_type(event_json) = 'object'
    AND length(event_json) <= 32768
  ),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','complete','dead_letter')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'pending' AND completed_at_ms IS NULL)
    OR (state IN ('complete','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_analytics_event_video_time_v1
  ON legacy_analytics_event_outbox_v1(video_id, created_at_ms, operation_id);

CREATE TABLE legacy_analytics_email_outbox_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_analytics_provider_operations_v1(operation_id) ON DELETE RESTRICT,
  video_id TEXT NOT NULL UNIQUE REFERENCES videos(id) ON DELETE RESTRICT,
  recipient_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  recipient_email TEXT NOT NULL CHECK (length(recipient_email) BETWEEN 3 AND 320),
  viewer_user_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  viewer_name TEXT NOT NULL CHECK (length(viewer_name) BETWEEN 1 AND 256),
  is_anonymous INTEGER NOT NULL CHECK (is_anonymous IN (0,1)),
  payload_json TEXT NOT NULL CHECK (
    json_valid(payload_json) AND json_type(payload_json) = 'object'
    AND length(payload_json) <= 32768
  ),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','complete','dead_letter')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (is_anonymous = 1 AND viewer_user_id IS NULL)
    OR (is_anonymous = 0 AND viewer_user_id IS NOT NULL)
  ),
  CHECK (
    (state = 'pending' AND completed_at_ms IS NULL)
    OR (state IN ('complete','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_analytics_notification_outbox_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_analytics_provider_operations_v1(operation_id) ON DELETE RESTRICT,
  deduplication_key TEXT NOT NULL UNIQUE CHECK (length(deduplication_key) BETWEEN 1 AND 512),
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  recipient_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  anonymous_name TEXT NOT NULL CHECK (length(anonymous_name) BETWEEN 1 AND 256),
  location TEXT CHECK (location IS NULL OR length(location) <= 513),
  payload_json TEXT NOT NULL CHECK (
    json_valid(payload_json) AND json_type(payload_json) = 'object'
    AND length(payload_json) <= 32768
  ),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','complete','dead_letter')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'pending' AND completed_at_ms IS NULL)
    OR (state IN ('complete','dead_letter') AND completed_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_analytics_notification_rate_v1
  ON legacy_analytics_notification_outbox_v1(
    recipient_user_id, video_id, created_at_ms DESC
  );

-- Signup tracking is provider-free. The checked-in statement performs the
-- source action's JSON preference compare-and-swap directly on `users`; the
-- preference value itself is the durable claim, so no shadow receipt can
-- incorrectly suppress a later CAS if another settings write removes it.

CREATE TABLE legacy_analytics_audit_v1 (
  audit_id TEXT PRIMARY KEY NOT NULL CHECK (length(audit_id) = 36),
  operation_id TEXT UNIQUE
    REFERENCES legacy_analytics_provider_operations_v1(operation_id) ON DELETE RESTRICT,
  source_operation_id TEXT NOT NULL,
  principal_digest TEXT NOT NULL CHECK (length(principal_digest) = 64),
  target_digest TEXT NOT NULL CHECK (length(target_digest) = 64),
  request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
  result_kind TEXT NOT NULL CHECK (result_kind IN (
    'provider_query_pending','provider_event_pending','track_skipped','signup_claimed','signup_ignored'
  )),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER legacy_analytics_notification_rate_guard_v1
BEFORE INSERT ON legacy_analytics_notification_outbox_v1
WHEN (
  SELECT COUNT(*) FROM legacy_analytics_notification_outbox_v1 existing
  WHERE existing.recipient_user_id = NEW.recipient_user_id
    AND existing.video_id = NEW.video_id
    AND existing.created_at_ms >= NEW.created_at_ms - 300000
) >= 50
BEGIN SELECT RAISE(IGNORE); END;

CREATE TRIGGER legacy_analytics_provider_operation_guard_v1
BEFORE UPDATE ON legacy_analytics_provider_operations_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.principal_digest = NEW.principal_digest
  AND OLD.actor_id IS NEW.actor_id
  AND OLD.active_organization_id IS NEW.active_organization_id
  AND OLD.target_video_id IS NEW.target_video_id
  AND OLD.execution_key_digest = NEW.execution_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'pending'
  AND NEW.state IN ('complete','dead_letter')
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_operation_immutable_v1'); END;

CREATE TRIGGER legacy_analytics_provider_operation_no_delete_v1
BEFORE DELETE ON legacy_analytics_provider_operations_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_operation_immutable_v1'); END;

CREATE TRIGGER legacy_analytics_query_outbox_guard_v1
BEFORE UPDATE ON legacy_analytics_query_outbox_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.query_kind = NEW.query_kind
  AND OLD.request_json = NEW.request_json
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'pending'
  AND NEW.attempt_count = OLD.attempt_count + 1
  AND (
    (NEW.state = 'pending' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state IN ('complete','dead_letter') AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_query_outbox_no_delete_v1
BEFORE DELETE ON legacy_analytics_query_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_analytics_event_outbox_guard_v1
BEFORE UPDATE ON legacy_analytics_event_outbox_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.timestamp_iso = NEW.timestamp_iso
  AND OLD.session_id = NEW.session_id
  AND OLD.action = NEW.action
  AND OLD.version = NEW.version
  AND OLD.tenant_id = NEW.tenant_id
  AND OLD.video_id = NEW.video_id
  AND OLD.pathname = NEW.pathname
  AND OLD.country = NEW.country
  AND OLD.region = NEW.region
  AND OLD.city = NEW.city
  AND OLD.browser = NEW.browser
  AND OLD.device = NEW.device
  AND OLD.operating_system = NEW.operating_system
  AND OLD.raw_user_agent = NEW.raw_user_agent
  AND OLD.user_id IS NEW.user_id
  AND OLD.event_json = NEW.event_json
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'pending'
  AND NEW.attempt_count = OLD.attempt_count + 1
  AND (
    (NEW.state = 'pending' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state IN ('complete','dead_letter') AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_event_outbox_no_delete_v1
BEFORE DELETE ON legacy_analytics_event_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_analytics_email_outbox_guard_v1
BEFORE UPDATE ON legacy_analytics_email_outbox_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.video_id = NEW.video_id
  AND OLD.recipient_user_id = NEW.recipient_user_id
  AND OLD.recipient_email = NEW.recipient_email
  AND OLD.viewer_user_id IS NEW.viewer_user_id
  AND OLD.viewer_name = NEW.viewer_name
  AND OLD.is_anonymous = NEW.is_anonymous
  AND OLD.payload_json = NEW.payload_json
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'pending'
  AND NEW.attempt_count = OLD.attempt_count + 1
  AND (
    (NEW.state = 'pending' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state IN ('complete','dead_letter') AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_email_outbox_no_delete_v1
BEFORE DELETE ON legacy_analytics_email_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_notification_outbox_guard_v1
BEFORE UPDATE ON legacy_analytics_notification_outbox_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.deduplication_key = NEW.deduplication_key
  AND OLD.video_id = NEW.video_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.recipient_user_id = NEW.recipient_user_id
  AND OLD.anonymous_name = NEW.anonymous_name
  AND OLD.location IS NEW.location
  AND OLD.payload_json = NEW.payload_json
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'pending'
  AND NEW.attempt_count = OLD.attempt_count + 1
  AND (
    (NEW.state = 'pending' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state IN ('complete','dead_letter') AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_notification_outbox_no_delete_v1
BEFORE DELETE ON legacy_analytics_notification_outbox_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_outbox_immutable_v1'); END;

CREATE TRIGGER legacy_analytics_password_grant_no_update_v1
BEFORE UPDATE ON legacy_analytics_password_grants_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_grant_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_password_grant_no_delete_v1
BEFORE DELETE ON legacy_analytics_password_grants_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_grant_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_audit_no_update_v1
BEFORE UPDATE ON legacy_analytics_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_audit_immutable_v1'); END;
CREATE TRIGGER legacy_analytics_audit_no_delete_v1
BEFORE DELETE ON legacy_analytics_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_analytics_audit_immutable_v1'); END;
