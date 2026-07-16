PRAGMA foreign_keys = ON;

CREATE TABLE users (
  id TEXT PRIMARY KEY NOT NULL,
  email TEXT NOT NULL COLLATE NOCASE UNIQUE,
  display_name TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE TABLE videos (
  id TEXT PRIMARY KEY NOT NULL,
  owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  title TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pending', 'uploading', 'processing', 'ready', 'failed', 'deleted')),
  source_object_key TEXT,
  playback_object_key TEXT,
  duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE INDEX videos_owner_created_idx ON videos(owner_id, created_at_ms DESC);
CREATE INDEX videos_state_updated_idx ON videos(state, updated_at_ms);

CREATE TABLE media_jobs (
  id TEXT PRIMARY KEY NOT NULL,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('queued', 'leased', 'running', 'succeeded', 'failed', 'cancelled')),
  idempotency_key TEXT NOT NULL UNIQUE,
  attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
  payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
  error_code TEXT,
  lease_expires_at_ms INTEGER,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE INDEX media_jobs_state_created_idx ON media_jobs(state, created_at_ms);

CREATE TABLE object_manifests (
  object_key TEXT PRIMARY KEY NOT NULL,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  bytes INTEGER NOT NULL CHECK (bytes >= 0),
  checksum_sha256 TEXT,
  content_type TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX object_manifests_video_idx ON object_manifests(video_id, role);
