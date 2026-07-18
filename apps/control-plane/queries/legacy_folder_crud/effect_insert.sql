INSERT INTO legacy_folder_crud_effects_v1(
  operation_id, organization_id, actor_id, mutation_kind, scope_kind,
  scope_id, invalidation_json, affected_folder_count, created_at_ms
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
