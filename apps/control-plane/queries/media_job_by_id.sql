SELECT j.id,
       j.state,
       json_extract(j.payload_json, '$.profile') AS profile,
       j.selected_executor,
       j.progress_basis_points,
       j.attempt,
       j.cancel_requested,
       j.error_class,
       j.created_at_ms,
       j.updated_at_ms
FROM media_jobs j
JOIN videos v
  ON v.id = j.video_id
 AND v.organization_id = j.organization_id
WHERE j.id = ?1
  AND j.organization_id = ?2
LIMIT 1
