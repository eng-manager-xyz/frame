INSERT INTO legacy_video_property_assertions_v1 (
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'durable', 1,
  CASE WHEN
    (SELECT COUNT(*) FROM legacy_video_property_operations_v1 o
     WHERE o.operation_id = ?1 AND o.state = 'complete'
       AND o.completed_at_ms = ?2) = 1
    AND (SELECT COUNT(*) FROM legacy_video_property_receipts_v1 r
         WHERE r.operation_id = ?1 AND r.result_digest = ?3) = 1
    AND (SELECT COUNT(*) FROM legacy_video_property_audit_v1 a
         WHERE a.operation_id = ?1 AND a.result_digest = ?3) = 1
    AND (SELECT COUNT(*) FROM legacy_video_property_effects_v1 e
         WHERE e.operation_id = ?1) = ?4
  THEN 1 ELSE 0 END;
