PRAGMA foreign_keys = ON;

INSERT OR IGNORE INTO users (
  id, email, display_name, created_at_ms, updated_at_ms,
  status, session_version, email_verified_at_ms, preferences_json
) VALUES (
  'user_local_owner', 'owner@frame.invalid', 'Local Owner', 1700000000000, 1700000000000,
  'active', 0, 1700000000000, '{}'
);

INSERT OR IGNORE INTO organizations (
  id, owner_id, name, status, settings_json, created_at_ms, updated_at_ms, revision
) VALUES (
  'org_local', 'user_local_owner', 'Local Frame', 'active', '{}',
  1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO organization_members (
  organization_id, user_id, role, state, has_pro_seat,
  created_at_ms, updated_at_ms, revision
) VALUES (
  'org_local', 'user_local_owner', 'owner', 'active', 1,
  1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO spaces (
  id, organization_id, created_by_user_id, name, is_primary, is_public,
  settings_json, created_at_ms, updated_at_ms, revision
) VALUES (
  'space_local', 'org_local', 'user_local_owner', 'Recordings', 1, 0,
  '{}', 1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO videos (
  id, owner_id, title, state, source_object_key, playback_object_key,
  duration_ms, created_at_ms, updated_at_ms, organization_id,
  privacy, metadata_json, revision
) VALUES (
  'video_local_ready', 'user_local_owner', 'Synthetic local fixture', 'ready',
  'tenants/org_local/videos/video_local_ready/source/v1/source.webm',
  'tenants/org_local/videos/video_local_ready/derivatives/playback/v1/video.webm',
  2000, 1700000000000, 1700000000000, 'org_local', 'public',
  '{"fixture":true}', 1
);

INSERT OR IGNORE INTO space_videos (
  space_id, video_id, added_by_user_id, added_at_ms
) VALUES (
  'space_local', 'video_local_ready', 'user_local_owner', 1700000000000
);

INSERT OR IGNORE INTO storage_integrations (
  id, organization_id, owner_user_id, provider, state, capabilities_json,
  created_at_ms, updated_at_ms, revision
) VALUES (
  'storage_local_r2', 'org_local', 'user_local_owner', 'r2', 'active',
  '{"conditional_put":true,"multipart":true,"range":true}',
  1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO storage_objects (
  id, organization_id, integration_id, video_id, object_key, role,
  object_version, state, bytes, content_type, checksum_sha256, provider_etag,
  created_at_ms
) VALUES (
  'object_local_source', 'org_local', 'storage_local_r2', 'video_local_ready',
  'tenants/org_local/videos/video_local_ready/source/v1/source.webm', 'source',
  1, 'available', 4096, 'video/webm',
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'local-source-v1', 1700000000000
);

INSERT OR IGNORE INTO object_manifests (
  object_key, video_id, role, bytes, checksum_sha256, content_type,
  created_at_ms, organization_id, object_version, provider_etag, state,
  updated_at_ms
) VALUES (
  'tenants/org_local/videos/video_local_ready/source/v1/source.webm',
  'video_local_ready', 'source', 4096,
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'video/webm', 1700000000000, 'org_local', 1, 'local-source-v1',
  'available', 1700000000000
);

INSERT OR IGNORE INTO media_jobs (
  id, video_id, kind, state, idempotency_key, attempt, payload_json,
  created_at_ms, updated_at_ms, organization_id, selected_executor,
  source_version, profile_version, output_object_key, progress_basis_points,
  revision
) VALUES (
  'job_local_thumbnail', 'video_local_ready', 'frame', 'succeeded',
  'local-seed-thumbnail-v1', 1, '{"synthetic":true}',
  1700000000000, 1700000000000, 'org_local', 'cloudflare_media',
  1, 1,
  'tenants/org_local/videos/video_local_ready/derivatives/frame/v1/thumbnail.jpg',
  10000, 1
);
