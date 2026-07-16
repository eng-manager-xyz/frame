PRAGMA foreign_keys = ON;

INSERT OR IGNORE INTO users (
  id, email, display_name, created_at_ms, updated_at_ms,
  status, session_version, email_verified_at_ms, preferences_json
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900101', 'owner@frame.invalid', 'Local Owner', 1700000000000, 1700000000000,
  'active', 0, 1700000000000, '{}'
);

INSERT OR IGNORE INTO organizations (
  id, owner_id, name, status, settings_json, created_at_ms, updated_at_ms, revision
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900102', '018f47a6-7b1c-7f55-8f39-8f8a86900101', 'Local Frame', 'active', '{}',
  1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO organization_members (
  organization_id, user_id, role, state, has_pro_seat,
  created_at_ms, updated_at_ms, revision
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900102', '018f47a6-7b1c-7f55-8f39-8f8a86900101', 'owner', 'active', 1,
  1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO spaces (
  id, organization_id, created_by_user_id, name, is_primary, is_public,
  settings_json, created_at_ms, updated_at_ms, revision
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900103', '018f47a6-7b1c-7f55-8f39-8f8a86900102', '018f47a6-7b1c-7f55-8f39-8f8a86900101', 'Recordings', 1, 0,
  '{}', 1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO videos (
  id, owner_id, title, state, source_object_key, playback_object_key,
  duration_ms, created_at_ms, updated_at_ms, organization_id,
  privacy, metadata_json, revision
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900104', '018f47a6-7b1c-7f55-8f39-8f8a86900101', 'Synthetic local fixture', 'ready',
  'tenants/018f47a6-7b1c-7f55-8f39-8f8a86900102/videos/018f47a6-7b1c-7f55-8f39-8f8a86900104/source/v1/source.webm',
  NULL,
  2000, 1700000000000, 1700000000000, '018f47a6-7b1c-7f55-8f39-8f8a86900102', 'private',
  '{"fixture":true}', 1
);

INSERT OR IGNORE INTO space_videos (
  space_id, video_id, added_by_user_id, added_at_ms
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900103', '018f47a6-7b1c-7f55-8f39-8f8a86900104', '018f47a6-7b1c-7f55-8f39-8f8a86900101', 1700000000000
);

INSERT OR IGNORE INTO storage_integrations (
  id, organization_id, owner_user_id, provider, state, capabilities_json,
  created_at_ms, updated_at_ms, revision
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900105', '018f47a6-7b1c-7f55-8f39-8f8a86900102', '018f47a6-7b1c-7f55-8f39-8f8a86900101', 'r2', 'active',
  '{"conditional_put":true,"multipart":true,"range":true}',
  1700000000000, 1700000000000, 0
);

INSERT OR IGNORE INTO storage_objects (
  id, organization_id, integration_id, video_id, object_key, role,
  object_version, state, bytes, content_type, checksum_sha256, provider_etag,
  created_at_ms
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900106', '018f47a6-7b1c-7f55-8f39-8f8a86900102', '018f47a6-7b1c-7f55-8f39-8f8a86900105', '018f47a6-7b1c-7f55-8f39-8f8a86900104',
  'tenants/018f47a6-7b1c-7f55-8f39-8f8a86900102/videos/018f47a6-7b1c-7f55-8f39-8f8a86900104/source/v1/source.webm', 'source',
  1, 'available', 4096, 'video/webm',
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'local-source-v1', 1700000000000
);

INSERT OR IGNORE INTO object_manifests (
  object_key, video_id, role, bytes, checksum_sha256, content_type,
  created_at_ms, organization_id, object_version, provider_etag, state,
  updated_at_ms
) VALUES (
  'tenants/018f47a6-7b1c-7f55-8f39-8f8a86900102/videos/018f47a6-7b1c-7f55-8f39-8f8a86900104/source/v1/source.webm',
  '018f47a6-7b1c-7f55-8f39-8f8a86900104', 'source', 4096,
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'video/webm', 1700000000000, '018f47a6-7b1c-7f55-8f39-8f8a86900102', 1, 'local-source-v1',
  'available', 1700000000000
);

INSERT OR IGNORE INTO media_jobs (
  id, video_id, kind, state, idempotency_key, attempt, payload_json,
  created_at_ms, updated_at_ms, organization_id, selected_executor,
  source_version, profile_version, output_object_key, progress_basis_points,
  revision
) VALUES (
  '018f47a6-7b1c-7f55-8f39-8f8a86900107', '018f47a6-7b1c-7f55-8f39-8f8a86900104', 'frame', 'succeeded',
  'local-seed-thumbnail-v1', 1, '{"synthetic":true}',
  1700000000000, 1700000000000, '018f47a6-7b1c-7f55-8f39-8f8a86900102', 'cloudflare_media',
  1, 1,
  'tenants/018f47a6-7b1c-7f55-8f39-8f8a86900102/videos/018f47a6-7b1c-7f55-8f39-8f8a86900104/derivatives/frame/v1/thumbnail.jpg',
  10000, 1
);
