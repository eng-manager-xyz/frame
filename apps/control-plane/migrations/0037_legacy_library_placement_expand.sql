PRAGMA foreign_keys = ON;

-- Cover the exact tenant/scope lookups used by the provider-free atomic
-- library-placement adapter.  This migration is intentionally expand-only:
-- 0036 introduced the normalized revision columns and transition guards, and
-- this step adds no second authority or placement ledger.
CREATE INDEX legacy_library_placement_shared_snapshot_v1
  ON shared_videos(organization_id, video_id, revoked_at_ms, folder_id, revision);

CREATE INDEX legacy_library_placement_space_snapshot_v1
  ON space_videos(space_id, video_id, folder_id, revision);

CREATE INDEX legacy_library_placement_direct_folder_v1
  ON videos(organization_id, id, folder_id, revision)
  WHERE deleted_at_ms IS NULL;
