INSERT INTO legacy_collaboration_effects_v1(
  operation_id, notification_timing, notification_failure_rolls_back_core,
  revalidation_path, created_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5);
