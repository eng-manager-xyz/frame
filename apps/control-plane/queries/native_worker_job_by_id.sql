SELECT j.id,
       j.video_id,
       j.state,
       j.revision,
       j.attempt,
       json_extract(j.payload_json, '$.profile') AS profile,
       j.source_version,
       j.output_object_key,
       j.worker_id,
       j.lease_token_digest,
       j.lease_expires_at_ms,
       j.progress_basis_points,
       j.cancel_requested
FROM media_jobs j
JOIN videos v
  ON v.id = j.video_id
 AND v.organization_id = j.organization_id
WHERE j.id = ?1
  AND j.organization_id = ?2
  AND j.selected_executor = 'native_gstreamer'
LIMIT 1
