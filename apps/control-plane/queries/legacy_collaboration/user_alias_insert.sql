INSERT OR IGNORE INTO legacy_collaboration_user_aliases_v1(
  legacy_user_id, mapped_user_id, image_url, provenance, created_at_ms, refreshed_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?5);
