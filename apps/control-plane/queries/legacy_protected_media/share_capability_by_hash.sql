WITH target AS (
  SELECT alias.mapped_video_id, alias.legacy_video_id
  FROM legacy_collaboration_video_aliases_v1 alias
  JOIN videos video ON video.id = alias.mapped_video_id
  WHERE alias.legacy_video_id = ?1
    AND video.deleted_at_ms IS NULL
), candidates AS (
  SELECT
    'video_password_capability' AS capability_kind,
    0 AS capability_ordinal,
    video.id AS capability_subject_id,
    video.legacy_property_revision AS capability_revision,
    video.legacy_password_hash AS password_hash
  FROM target
  JOIN videos video ON video.id = target.mapped_video_id
  WHERE video.legacy_password_hash = ?2

  UNION ALL

  SELECT
    'space_password_capability' AS capability_kind,
    1 AS capability_ordinal,
    space.id AS capability_subject_id,
    space.legacy_password_revision AS capability_revision,
    space.legacy_password_hash AS password_hash
  FROM target
  JOIN space_videos placement ON placement.video_id = target.mapped_video_id
  JOIN spaces space ON space.id = placement.space_id
  WHERE space.deleted_at_ms IS NULL
    AND space.legacy_password_hash = ?2
)
SELECT capability_kind, capability_ordinal, capability_subject_id,
       capability_revision, password_hash
FROM candidates
ORDER BY capability_ordinal, capability_subject_id
LIMIT 2;
