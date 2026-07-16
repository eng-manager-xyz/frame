PRAGMA foreign_keys = ON;

-- Enforce tenant-bound business documents and scoped collaboration writes in
-- a bounded D1 migration after the authority expansion has completed.

CREATE TRIGGER videos_document_contract_v1
BEFORE INSERT ON videos
WHEN NEW.metadata_json IS NOT NULL AND (
  length(NEW.metadata_json) > 65536 OR json_valid(NEW.metadata_json) = 0
  OR json_extract(NEW.metadata_json, '$.schema_version') <> NEW.metadata_schema_version
  OR NEW.metadata_checksum IS NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_document_invalid_v1');
END;
CREATE TRIGGER videos_document_update_contract_v1
BEFORE UPDATE OF metadata_json, metadata_schema_version, metadata_checksum ON videos
WHEN NEW.metadata_json IS NOT NULL AND (
  length(NEW.metadata_json) > 65536 OR json_valid(NEW.metadata_json) = 0
  OR json_extract(NEW.metadata_json, '$.schema_version') <> NEW.metadata_schema_version
  OR NEW.metadata_checksum IS NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_document_invalid_v1');
END;

CREATE TRIGGER video_edits_document_contract_v1
BEFORE INSERT ON video_edits
WHEN length(NEW.edit_spec_json) > 1048576
  OR json_extract(NEW.edit_spec_json, '$.schema_version') <> NEW.document_version
  OR NEW.document_checksum IS NULL
BEGIN
  SELECT RAISE(ABORT, 'frame_business_document_invalid_v1');
END;

CREATE TRIGGER shared_videos_scope_policy_v1
BEFORE INSERT ON shared_videos
WHEN NOT EXISTS (
  SELECT 1 FROM videos v
  JOIN organization_members m
    ON m.organization_id = v.organization_id AND m.user_id = NEW.shared_by_user_id
  WHERE v.id = NEW.video_id
    AND v.organization_id = NEW.organization_id
    AND v.deleted_at_ms IS NULL
    AND m.state = 'active'
    AND (m.role IN ('owner','admin') OR v.owner_id = NEW.shared_by_user_id)
    AND ((NEW.sharing_mode = 'space') = (NEW.folder_id IS NOT NULL))
    AND (NEW.folder_id IS NULL OR EXISTS (
      SELECT 1 FROM folders folder
      WHERE folder.id = NEW.folder_id AND folder.organization_id = NEW.organization_id
        AND folder.deleted_at_ms IS NULL
    ))
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER comments_scope_policy_v1
BEFORE INSERT ON comments
WHEN NEW.parent_comment_id = NEW.id
  OR (NEW.parent_comment_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM comments parent
    WHERE parent.id = NEW.parent_comment_id
      AND parent.video_id = NEW.video_id
      AND parent.organization_id = NEW.organization_id
      AND parent.deleted_at_ms IS NULL
  ))
  OR (NEW.comment_kind = 'emoji' AND (
    length(NEW.body) > 16 OR instr(NEW.body, ' ') > 0
    OR instr(NEW.body, char(9)) > 0 OR instr(NEW.body, char(10)) > 0
    OR instr(NEW.body, char(13)) > 0
  ))
  OR NOT EXISTS (
  SELECT 1 FROM videos v
  WHERE v.id = NEW.video_id
    AND v.organization_id = NEW.organization_id
    AND v.deleted_at_ms IS NULL
    AND v.comments_enabled = 1
    AND (
      (NEW.author_user_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM organization_members m
        WHERE m.organization_id = v.organization_id
          AND m.user_id = NEW.author_user_id AND m.state = 'active'
      ))
      OR (
        NEW.anonymous_author_digest IS NOT NULL
        AND v.privacy IN ('public','unlisted')
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER notifications_scope_policy_v1
BEFORE INSERT ON notifications
WHEN NEW.type NOT IN ('view','comment','reply','reaction','anon_view')
  OR NEW.organization_id IS NULL OR NOT EXISTS (
  SELECT 1 FROM organization_members recipient
  WHERE recipient.organization_id = NEW.organization_id
    AND recipient.user_id = NEW.recipient_user_id
    AND recipient.state = 'active'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER storage_integrations_scope_policy_v1
BEFORE INSERT ON storage_integrations
WHEN NEW.credential_ciphertext IS NULL
  OR NEW.capabilities_checksum IS NULL
  OR json_extract(NEW.capabilities_json, '$.schema_version') <> NEW.capabilities_schema_version
  OR (NEW.owner_user_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM organization_members member
    WHERE member.organization_id = NEW.organization_id
      AND member.user_id = NEW.owner_user_id AND member.state = 'active'
  ))
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER developer_apps_scope_policy_v1
BEFORE INSERT ON developer_apps
WHEN NEW.organization_id IS NULL OR NOT EXISTS (
  SELECT 1 FROM organization_members member
  WHERE member.organization_id = NEW.organization_id
    AND member.user_id = NEW.owner_user_id AND member.state = 'active'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;

CREATE TRIGGER developer_videos_scope_policy_v1
BEFORE INSERT ON developer_videos
WHEN NEW.external_user_digest IS NULL
  OR NEW.external_user_id <> NEW.external_user_digest
  OR (
    (NEW.metadata_json IS NULL) <> (NEW.metadata_checksum IS NULL)
    OR (NEW.metadata_json IS NOT NULL AND (
      length(NEW.metadata_json) > 65536
      OR json_extract(NEW.metadata_json, '$.schema_version') <> NEW.metadata_schema_version
    ))
  )
  OR NOT EXISTS (
    SELECT 1 FROM developer_apps app
    WHERE app.id = NEW.app_id AND app.organization_id IS NOT NULL
  )
  OR (NEW.video_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM videos video JOIN developer_apps app ON app.id = NEW.app_id
    WHERE video.id = NEW.video_id AND video.organization_id = app.organization_id
  ))
BEGIN
  SELECT RAISE(ABORT, 'frame_business_authority_conflict_v1');
END;
