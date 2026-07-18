PRAGMA foreign_keys = ON;

-- Source NanoIDs are not reversible from Frame UUIDs. Organization and video
-- aliases already exist in the preceding account/collaboration expands; this
-- final missing scope alias makes the three library membership reads lossless
-- without introducing a second business authority.
CREATE TABLE legacy_library_space_aliases_v1 (
  legacy_space_id TEXT PRIMARY KEY NOT NULL CHECK (
    length(legacy_space_id) = 15
    AND legacy_space_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  space_id TEXT NOT NULL UNIQUE REFERENCES spaces(id) ON DELETE CASCADE,
  provenance TEXT NOT NULL CHECK (
    provenance IN ('cap_backfill', 'native_generated')
  ),
  created_at_ms INTEGER NOT NULL CHECK (
    created_at_ms BETWEEN 0 AND 9007199254740991
  )
);

CREATE TRIGGER legacy_library_space_aliases_no_update_v1
BEFORE UPDATE ON legacy_library_space_aliases_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_library_alias_immutable_v1');
END;

CREATE INDEX legacy_library_shared_folder_read_v1
  ON shared_videos(folder_id, revoked_at_ms, video_id);
CREATE INDEX legacy_library_shared_root_read_v1
  ON shared_videos(organization_id, folder_id, revoked_at_ms, video_id);
CREATE INDEX legacy_library_space_folder_read_v1
  ON space_videos(space_id, folder_id, video_id);
CREATE INDEX legacy_library_folder_alias_scope_read_v1
  ON folders(legacy_folder_id, organization_id, legacy_scope_kind, legacy_scope_id, deleted_at_ms);
