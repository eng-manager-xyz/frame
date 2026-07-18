UPDATE legacy_mobile_cap_media_v1
SET transcription_status = NULL,
    updated_at_ms = CASE WHEN updated_at_ms < ?2 THEN ?2 ELSE updated_at_ms END
WHERE mapped_video_id = ?1
  AND owner_id = ?3;
