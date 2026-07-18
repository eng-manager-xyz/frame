UPDATE videos
SET legacy_public = COALESCE(?2, legacy_public),
    privacy = CASE WHEN ?2 IS NULL THEN privacy WHEN ?2 = 1 THEN 'public' ELSE 'private' END,
    updated_at_ms = CASE WHEN updated_at_ms < ?3 THEN ?3 ELSE updated_at_ms END,
    revision = revision + CASE WHEN ?2 IS NULL THEN 0 ELSE 1 END,
    last_operation_id = ?4
WHERE id = ?1;
