INSERT INTO videos(
  id, owner_id, title, state, duration_ms, created_at_ms, updated_at_ms,
  organization_id, folder_id, privacy, revision, last_operation_id,
  legacy_public, legacy_duration_seconds, legacy_instant_recording,
  legacy_instant_resolution, legacy_instant_width, legacy_instant_height,
  legacy_instant_video_codec, legacy_instant_audio_codec,
  legacy_instant_supports_progress
) VALUES (
  ?1, ?2, ?3, 'uploading', ?4, ?5, ?5,
  ?6, ?7, ?8, 0, ?9,
  ?10, ?11, 1, ?12, ?13, ?14, ?15, ?16, ?17
);
