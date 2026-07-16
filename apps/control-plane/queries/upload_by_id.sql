SELECT u.id,
       u.organization_id,
       u.video_id,
       u.state,
       u.expected_bytes,
       u.received_bytes,
       u.source_object_key,
       u.source_version,
       u.content_type,
       u.checksum_sha256,
       u.transfer_mode,
       u.direct_staging_key,
       u.direct_checksum_sha256,
       u.direct_expires_at_ms
FROM video_uploads u
JOIN videos v
  ON v.id = u.video_id
 AND v.organization_id = u.organization_id
WHERE u.id = ?1
  AND u.organization_id = ?2
LIMIT 1
