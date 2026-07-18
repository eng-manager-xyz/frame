PRAGMA foreign_keys = ON;

CREATE TABLE organizations (
  id TEXT PRIMARY KEY NOT NULL,
  owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 160),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'tombstoned', 'deleted')),
  settings_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(settings_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  tombstoned_at_ms INTEGER CHECK (tombstoned_at_ms IS NULL OR tombstoned_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991)
);
CREATE INDEX organizations_owner_status_idx ON organizations(owner_id, status);

CREATE TABLE organization_members (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'suspended', 'removed')),
  has_pro_seat INTEGER NOT NULL DEFAULT 0 CHECK (has_pro_seat IN (0, 1)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX organization_members_user_state_idx ON organization_members(user_id, state);

CREATE TABLE organization_invites (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  invited_email_digest TEXT NOT NULL CHECK (length(invited_email_digest) = 64),
  invited_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'viewer')),
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'declined', 'revoked', 'expired')),
  token_digest TEXT NOT NULL UNIQUE CHECK (length(token_digest) = 64),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  resolved_at_ms INTEGER CHECK (resolved_at_ms IS NULL OR resolved_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  CHECK (expires_at_ms > created_at_ms)
);
CREATE INDEX organization_invites_org_status_idx ON organization_invites(organization_id, status);
CREATE INDEX organization_invites_email_status_idx ON organization_invites(invited_email_digest, status);

CREATE TABLE organization_allowed_domains (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  domain_ascii TEXT NOT NULL COLLATE NOCASE,
  verified_at_ms INTEGER CHECK (verified_at_ms IS NULL OR verified_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, domain_ascii)
);

CREATE TABLE spaces (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 160),
  is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
  is_public INTEGER NOT NULL DEFAULT 0 CHECK (is_public IN (0, 1)),
  settings_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(settings_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991)
);
CREATE INDEX spaces_org_created_idx ON spaces(organization_id, created_at_ms DESC);
CREATE UNIQUE INDEX spaces_one_primary_per_org_idx
  ON spaces(organization_id) WHERE is_primary = 1 AND deleted_at_ms IS NULL;

CREATE TABLE space_members (
  space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('manager', 'contributor', 'viewer')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (space_id, user_id)
);
CREATE INDEX space_members_user_idx ON space_members(user_id, space_id);

CREATE TABLE folders (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  space_id TEXT REFERENCES spaces(id) ON DELETE CASCADE,
  parent_id TEXT REFERENCES folders(id) ON DELETE CASCADE,
  created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 255),
  is_public INTEGER NOT NULL DEFAULT 0 CHECK (is_public IN (0, 1)),
  settings_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(settings_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  CHECK (parent_id IS NULL OR parent_id <> id)
);
CREATE INDEX folders_org_parent_idx ON folders(organization_id, parent_id, created_at_ms);
CREATE INDEX folders_space_parent_idx ON folders(space_id, parent_id, created_at_ms);

-- Nullable expansion columns allow existing scaffold rows to be backfilled before enforcement.
ALTER TABLE videos ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT;
ALTER TABLE videos ADD COLUMN folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL;
ALTER TABLE videos ADD COLUMN privacy TEXT NOT NULL DEFAULT 'private'
  CHECK (privacy IN ('private', 'organization', 'public', 'unlisted'));
ALTER TABLE videos ADD COLUMN metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json));
ALTER TABLE videos ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision >= 0 AND revision <= 9007199254740991);
ALTER TABLE videos ADD COLUMN deleted_at_ms INTEGER
  CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991);
CREATE INDEX videos_org_created_idx ON videos(organization_id, created_at_ms DESC);
CREATE INDEX videos_folder_created_idx ON videos(folder_id, created_at_ms DESC);

CREATE TABLE space_videos (
  space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
  added_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  added_at_ms INTEGER NOT NULL CHECK (added_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (space_id, video_id)
);
CREATE INDEX space_videos_folder_idx ON space_videos(space_id, folder_id, added_at_ms DESC);

CREATE TABLE video_edits (
  id TEXT PRIMARY KEY NOT NULL,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  document_version INTEGER NOT NULL CHECK (document_version > 0),
  edit_spec_json TEXT NOT NULL CHECK (json_valid(edit_spec_json) AND length(edit_spec_json) <= 1048576),
  created_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  UNIQUE (video_id, document_version)
);

CREATE TABLE shared_videos (
  id TEXT PRIMARY KEY NOT NULL,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
  shared_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  sharing_mode TEXT NOT NULL CHECK (sharing_mode IN ('organization', 'space', 'public_link')),
  shared_at_ms INTEGER NOT NULL CHECK (shared_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (video_id, organization_id, folder_id)
);
CREATE INDEX shared_videos_org_time_idx ON shared_videos(organization_id, shared_at_ms DESC);
CREATE UNIQUE INDEX shared_videos_org_without_folder_idx
  ON shared_videos(video_id, organization_id) WHERE folder_id IS NULL;

CREATE TABLE comments (
  id TEXT PRIMARY KEY NOT NULL,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  parent_comment_id TEXT REFERENCES comments(id) ON DELETE CASCADE,
  author_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  anonymous_author_digest TEXT CHECK (anonymous_author_digest IS NULL OR length(anonymous_author_digest) = 64),
  body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 10000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  CHECK ((author_user_id IS NOT NULL) <> (anonymous_author_digest IS NOT NULL))
);
CREATE INDEX comments_video_created_idx ON comments(video_id, created_at_ms);

CREATE TABLE notifications (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE,
  recipient_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  type TEXT NOT NULL CHECK (length(type) BETWEEN 1 AND 64),
  deduplication_key TEXT NOT NULL,
  data_json TEXT NOT NULL CHECK (json_valid(data_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  read_at_ms INTEGER CHECK (read_at_ms IS NULL OR read_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (recipient_user_id, deduplication_key)
);
CREATE INDEX notifications_recipient_unread_idx
  ON notifications(recipient_user_id, created_at_ms DESC) WHERE read_at_ms IS NULL;

CREATE TABLE messenger_conversations (
  id TEXT PRIMARY KEY NOT NULL,
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  anonymous_actor_digest TEXT CHECK (anonymous_actor_digest IS NULL OR length(anonymous_actor_digest) = 64),
  mode TEXT NOT NULL CHECK (mode IN ('bot', 'support', 'closed')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  last_message_at_ms INTEGER NOT NULL CHECK (last_message_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (user_id IS NOT NULL OR anonymous_actor_digest IS NOT NULL)
);
CREATE INDEX messenger_conversations_activity_idx ON messenger_conversations(mode, last_message_at_ms DESC);

CREATE TABLE messenger_messages (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES messenger_conversations(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'support', 'system')),
  body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 50000),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX messenger_messages_conversation_idx ON messenger_messages(conversation_id, created_at_ms);

CREATE TABLE messenger_support_emails (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES messenger_conversations(id) ON DELETE CASCADE,
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  provider_message_id TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL CHECK (status IN ('pending', 'sent', 'failed')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);
