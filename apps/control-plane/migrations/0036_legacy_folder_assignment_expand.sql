PRAGMA foreign_keys = ON;

-- Expand-only state needed to fence Cap's three legacy folder-assignment
-- actions. Existing rows remain readable and auditable; the transition
-- triggers below prevent new cross-tenant or cross-space edges without
-- requiring a destructive cleanup migration.
ALTER TABLE space_videos ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE space_videos ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

CREATE INDEX space_videos_video_scope_assignment_v1
  ON space_videos(video_id, space_id, revision);
CREATE INDEX shared_videos_current_assignment_v1
  ON shared_videos(organization_id, video_id, revoked_at_ms, id);

CREATE TRIGGER legacy_direct_folder_insert_scope_guard_v1
BEFORE INSERT ON videos
WHEN NEW.folder_id IS NOT NULL AND NOT EXISTS (
  SELECT 1
  FROM folders f
  LEFT JOIN spaces s
    ON s.id = f.space_id
   AND s.organization_id = f.organization_id
   AND s.deleted_at_ms IS NULL
  WHERE f.id = NEW.folder_id
    AND f.organization_id = NEW.organization_id
    AND f.deleted_at_ms IS NULL
    AND (f.space_id IS NULL OR s.id IS NOT NULL)
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_scope_v1');
END;

CREATE TRIGGER legacy_direct_folder_update_scope_guard_v1
BEFORE UPDATE OF organization_id, folder_id ON videos
WHEN NEW.folder_id IS NOT NULL AND NOT EXISTS (
  SELECT 1
  FROM folders f
  LEFT JOIN spaces s
    ON s.id = f.space_id
   AND s.organization_id = f.organization_id
   AND s.deleted_at_ms IS NULL
  WHERE f.id = NEW.folder_id
    AND f.organization_id = NEW.organization_id
    AND f.deleted_at_ms IS NULL
    AND (f.space_id IS NULL OR s.id IS NOT NULL)
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_scope_v1');
END;

CREATE TRIGGER legacy_space_video_insert_scope_guard_v1
BEFORE INSERT ON space_videos
WHEN NOT EXISTS (
  SELECT 1
  FROM spaces s
  JOIN videos v
    ON v.id = NEW.video_id
   AND v.organization_id = s.organization_id
   AND v.deleted_at_ms IS NULL
  WHERE s.id = NEW.space_id
    AND s.deleted_at_ms IS NULL
    AND (
      NEW.folder_id IS NULL OR EXISTS (
        SELECT 1
        FROM folders f
        WHERE f.id = NEW.folder_id
          AND f.organization_id = s.organization_id
          AND f.space_id = s.id
          AND f.deleted_at_ms IS NULL
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_scope_v1');
END;

CREATE TRIGGER legacy_space_video_update_scope_guard_v1
BEFORE UPDATE OF space_id, video_id, folder_id ON space_videos
WHEN NOT EXISTS (
  SELECT 1
  FROM spaces s
  JOIN videos v
    ON v.id = NEW.video_id
   AND v.organization_id = s.organization_id
   AND v.deleted_at_ms IS NULL
  WHERE s.id = NEW.space_id
    AND s.deleted_at_ms IS NULL
    AND (
      NEW.folder_id IS NULL OR EXISTS (
        SELECT 1
        FROM folders f
        WHERE f.id = NEW.folder_id
          AND f.organization_id = s.organization_id
          AND f.space_id = s.id
          AND f.deleted_at_ms IS NULL
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_scope_v1');
END;

CREATE TRIGGER legacy_shared_video_insert_scope_guard_v1
BEFORE INSERT ON shared_videos
-- The original business-data trigger owns every row that fails its established
-- authority/scope contract and its stable `frame_business_authority_conflict_v1`
-- marker. This later compatibility trigger fires only after that older
-- contract is satisfied and adds active-organization/space checks.
WHEN EXISTS (
  SELECT 1
  FROM videos legacy_video
  JOIN organization_members legacy_actor
    ON legacy_actor.organization_id = legacy_video.organization_id
   AND legacy_actor.user_id = NEW.shared_by_user_id
  WHERE legacy_video.id = NEW.video_id
    AND legacy_video.organization_id = NEW.organization_id
    AND legacy_video.deleted_at_ms IS NULL
    AND legacy_actor.state = 'active'
    AND (
      legacy_actor.role IN ('owner', 'admin')
      OR legacy_video.owner_id = NEW.shared_by_user_id
    )
    AND ((NEW.sharing_mode = 'space') = (NEW.folder_id IS NOT NULL))
    AND (
      NEW.folder_id IS NULL OR EXISTS (
        SELECT 1
        FROM folders legacy_folder
        WHERE legacy_folder.id = NEW.folder_id
          AND legacy_folder.organization_id = NEW.organization_id
          AND legacy_folder.deleted_at_ms IS NULL
      )
    )
)
AND NOT EXISTS (
  SELECT 1
  FROM organizations o
  JOIN videos v
    ON v.id = NEW.video_id
   AND v.organization_id = o.id
   AND v.deleted_at_ms IS NULL
  WHERE o.id = NEW.organization_id
    AND o.status = 'active'
    AND EXISTS (
      SELECT 1
      FROM organization_members actor
      WHERE actor.organization_id = o.id
        AND actor.user_id = NEW.shared_by_user_id
        AND actor.state = 'active'
        AND (actor.role IN ('owner', 'admin') OR v.owner_id = NEW.shared_by_user_id)
    )
    AND ((NEW.sharing_mode = 'space') = (NEW.folder_id IS NOT NULL))
    AND (
      NEW.folder_id IS NULL OR EXISTS (
        SELECT 1
        FROM folders f
        LEFT JOIN spaces s
          ON s.id = f.space_id
         AND s.organization_id = f.organization_id
         AND s.deleted_at_ms IS NULL
        WHERE f.id = NEW.folder_id
          AND f.organization_id = o.id
          AND f.deleted_at_ms IS NULL
          AND (f.space_id IS NULL OR s.id IS NOT NULL)
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_scope_v1');
END;

CREATE TRIGGER legacy_shared_video_update_scope_guard_v1
BEFORE UPDATE OF video_id, organization_id, folder_id, sharing_mode, revoked_at_ms ON shared_videos
WHEN NEW.revoked_at_ms IS NULL AND NOT EXISTS (
  SELECT 1
  FROM organizations o
  JOIN videos v
    ON v.id = NEW.video_id
   AND v.organization_id = o.id
   AND v.deleted_at_ms IS NULL
  WHERE o.id = NEW.organization_id
    AND o.status = 'active'
    AND ((NEW.sharing_mode = 'space') = (NEW.folder_id IS NOT NULL))
    AND (
      NEW.folder_id IS NULL OR EXISTS (
        SELECT 1
        FROM folders f
        LEFT JOIN spaces s
          ON s.id = f.space_id
         AND s.organization_id = f.organization_id
         AND s.deleted_at_ms IS NULL
        WHERE f.id = NEW.folder_id
          AND f.organization_id = o.id
          AND f.deleted_at_ms IS NULL
          AND (f.space_id IS NULL OR s.id IS NOT NULL)
      )
    )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_scope_v1');
END;

-- Do not add a partial unique index here: an imported tenant may already have
-- multiple active rows and must remain inspectable. These guards preserve a
-- clean tenant and stop a dirty tenant from acquiring another current row.
CREATE TRIGGER legacy_shared_video_one_current_insert_guard_v1
BEFORE INSERT ON shared_videos
WHEN NEW.revoked_at_ms IS NULL
  AND EXISTS (
    SELECT 1
    FROM organizations o
    JOIN videos v
      ON v.id = NEW.video_id
     AND v.organization_id = o.id
     AND v.deleted_at_ms IS NULL
    WHERE o.id = NEW.organization_id
      AND o.status = 'active'
      AND EXISTS (
        SELECT 1
        FROM organization_members actor
        WHERE actor.organization_id = o.id
          AND actor.user_id = NEW.shared_by_user_id
          AND actor.state = 'active'
          AND (actor.role IN ('owner', 'admin') OR v.owner_id = NEW.shared_by_user_id)
      )
      AND ((NEW.sharing_mode = 'space') = (NEW.folder_id IS NOT NULL))
      AND (
        NEW.folder_id IS NULL OR EXISTS (
          SELECT 1
          FROM folders f
          LEFT JOIN spaces s
            ON s.id = f.space_id
           AND s.organization_id = f.organization_id
           AND s.deleted_at_ms IS NULL
          WHERE f.id = NEW.folder_id
            AND f.organization_id = o.id
            AND f.deleted_at_ms IS NULL
            AND (f.space_id IS NULL OR s.id IS NOT NULL)
        )
      )
  )
  AND EXISTS (
    SELECT 1
    FROM shared_videos existing
    WHERE existing.organization_id = NEW.organization_id
      AND existing.video_id = NEW.video_id
      AND existing.revoked_at_ms IS NULL
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_multiplicity_v1');
END;

CREATE TRIGGER legacy_shared_video_one_current_update_guard_v1
BEFORE UPDATE OF organization_id, video_id, revoked_at_ms ON shared_videos
WHEN NEW.revoked_at_ms IS NULL
  AND EXISTS (
    SELECT 1
    FROM organizations o
    JOIN videos v
      ON v.id = NEW.video_id
     AND v.organization_id = o.id
     AND v.deleted_at_ms IS NULL
    WHERE o.id = NEW.organization_id
      AND o.status = 'active'
      AND ((NEW.sharing_mode = 'space') = (NEW.folder_id IS NOT NULL))
      AND (
        NEW.folder_id IS NULL OR EXISTS (
          SELECT 1
          FROM folders f
          LEFT JOIN spaces s
            ON s.id = f.space_id
           AND s.organization_id = f.organization_id
           AND s.deleted_at_ms IS NULL
          WHERE f.id = NEW.folder_id
            AND f.organization_id = o.id
            AND f.deleted_at_ms IS NULL
            AND (f.space_id IS NULL OR s.id IS NOT NULL)
        )
      )
  )
  AND NOT (
    OLD.organization_id = NEW.organization_id
    AND OLD.video_id = NEW.video_id
    AND OLD.revoked_at_ms IS NULL
  )
  AND EXISTS (
    SELECT 1
    FROM shared_videos existing
    WHERE existing.organization_id = NEW.organization_id
      AND existing.video_id = NEW.video_id
      AND existing.revoked_at_ms IS NULL
      AND existing.id <> OLD.id
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_assignment_multiplicity_v1');
END;
