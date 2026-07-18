INSERT INTO legacy_collaboration_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'video_authority', 1, COUNT(*)
FROM legacy_collaboration_video_aliases_v1 video_alias
JOIN videos video ON video.id = video_alias.mapped_video_id
WHERE video_alias.legacy_video_id = ?2
  AND video_alias.mapped_video_id = ?3
  AND video.owner_id = ?4
  AND video.revision = ?7
  AND video.deleted_at_ms IS NULL
  AND COALESCE((
    SELECT MAX(shared.revision)
    FROM shared_videos shared
    WHERE shared.video_id = video.id
      AND shared.organization_id = ?6
      AND shared.revoked_at_ms IS NULL
  ), -1) = ?8
  AND CASE WHEN video.owner_id = ?5 THEN 'owner' ELSE 'active_organization_share' END = ?9
  AND (
    video.owner_id = ?5
    OR EXISTS (
      SELECT 1 FROM shared_videos shared
      WHERE shared.video_id = video.id
        AND shared.organization_id = ?6
        AND shared.revoked_at_ms IS NULL
    )
  );
