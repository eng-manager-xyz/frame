UPDATE legacy_mobile_cap_media_v1
SET object_prefix = ?2,
    source_type = ?3,
    transcription_status = ?4,
    updated_at_ms = MAX(updated_at_ms, ?5)
WHERE mapped_video_id = ?1;
