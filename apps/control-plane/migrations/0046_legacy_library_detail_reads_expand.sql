PRAGMA foreign_keys = ON;

-- Cap stores screenshot state and a floating-point duration in seconds. The
-- retained Frame row has neither the screenshot bit nor a lossless duration
-- representation (its duration is integral milliseconds), so keep explicit
-- compatibility shadows for source imports and native dual writes.
ALTER TABLE videos ADD COLUMN legacy_is_screenshot INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_is_screenshot IN (0, 1));
ALTER TABLE videos ADD COLUMN legacy_duration_seconds REAL
  CHECK (
    legacy_duration_seconds IS NULL OR (
      legacy_duration_seconds >= -1.7976931348623157e308
      AND legacy_duration_seconds <= 1.7976931348623157e308
    )
  );

UPDATE videos
SET legacy_duration_seconds =
  CASE WHEN duration_ms IS NULL THEN NULL ELSE duration_ms / 1000.0 END;

-- Cap's `effectiveCreatedAt` is a stored generated column. Its two accepted
-- UTC forms are an integral-second timestamp or the same timestamp with one
-- to six fractional digits. Keep microseconds for source-exact ordering, then
-- derive the millisecond value used by JavaScript Date projections. Virtual
-- columns keep later metadata edits ordered correctly without a second
-- mutable authority.
ALTER TABLE videos ADD COLUMN legacy_effective_created_at_us INTEGER
GENERATED ALWAYS AS (
  COALESCE(
    CASE
      WHEN legacy_metadata_json IS NOT NULL
       AND json_type(legacy_metadata_json, '$.customCreatedAt') = 'text'
       AND (
         length(json_extract(legacy_metadata_json, '$.customCreatedAt')) = 20
         OR (
           length(json_extract(legacy_metadata_json, '$.customCreatedAt')) BETWEEN 22 AND 27
           AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 20, 1) = '.'
           AND substr(
             json_extract(legacy_metadata_json, '$.customCreatedAt'),
             21,
             length(json_extract(legacy_metadata_json, '$.customCreatedAt')) - 21
           ) NOT GLOB '*[^0-9]*'
         )
       )
       AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 5, 1) = '-'
       AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 8, 1) = '-'
       AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 11, 1) = 'T'
       AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 14, 1) = ':'
       AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 17, 1) = ':'
       AND substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), -1, 1) = 'Z'
       AND strftime(
         '%Y-%m-%dT%H:%M:%S',
         json_extract(legacy_metadata_json, '$.customCreatedAt')
       ) = substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 1, 19)
      THEN unixepoch(
        substr(json_extract(legacy_metadata_json, '$.customCreatedAt'), 1, 19) || 'Z'
      ) * 1000000 + CASE
        WHEN length(json_extract(legacy_metadata_json, '$.customCreatedAt')) = 20
          THEN 0
        ELSE CAST(substr(
          substr(
            json_extract(legacy_metadata_json, '$.customCreatedAt'),
            21,
            length(json_extract(legacy_metadata_json, '$.customCreatedAt')) - 21
          ) || '000000',
          1,
          6
        ) AS INTEGER)
      END
    END,
    created_at_ms * 1000
  )
) VIRTUAL;
ALTER TABLE videos ADD COLUMN legacy_effective_created_at_ms INTEGER
GENERATED ALWAYS AS (legacy_effective_created_at_us / 1000) VIRTUAL;

CREATE INDEX legacy_library_detail_owner_order_v1
  ON videos(owner_id, organization_id, deleted_at_ms, legacy_effective_created_at_us DESC);
CREATE INDEX legacy_library_detail_search_order_v1
  ON videos(organization_id, deleted_at_ms, legacy_effective_created_at_us DESC, title);
CREATE INDEX legacy_library_detail_upload_presence_v1
  ON video_uploads(video_id);
CREATE INDEX legacy_library_detail_comment_counts_v1
  ON legacy_collaboration_comments_v1(legacy_video_id, comment_kind, legacy_comment_id);
