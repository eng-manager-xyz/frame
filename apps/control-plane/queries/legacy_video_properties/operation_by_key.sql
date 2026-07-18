SELECT
  o.operation_id,
  o.request_digest,
  o.state,
  r.result_kind,
  r.result_json,
  r.result_digest,
  e.effect_kind,
  e.effect_json,
  (SELECT COUNT(*) FROM legacy_video_property_effects_v1 ec
   WHERE ec.operation_id = o.operation_id) AS effect_count,
  (SELECT COUNT(*) FROM legacy_video_property_audit_v1 a
   WHERE a.operation_id = o.operation_id) AS audit_count
FROM legacy_video_property_operations_v1 o
LEFT JOIN legacy_video_property_receipts_v1 r ON r.operation_id = o.operation_id
LEFT JOIN legacy_video_property_effects_v1 e ON e.operation_id = o.operation_id
WHERE o.source_operation_id = ?1
  AND o.principal_digest = ?2
  AND o.video_id = ?3
  AND o.idempotency_key_digest = ?4;
