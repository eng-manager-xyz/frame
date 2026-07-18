INSERT INTO legacy_collaboration_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
SELECT ?1, 'durable_receipt', 1, COUNT(*)
FROM legacy_collaboration_operations_v1 operation
JOIN legacy_collaboration_receipts_v1 receipt
  ON receipt.operation_id = operation.operation_id
JOIN legacy_collaboration_effects_v1 effect
  ON effect.operation_id = operation.operation_id
JOIN legacy_collaboration_audit_events_v1 audit
  ON audit.operation_id = operation.operation_id
WHERE operation.operation_id = ?1 AND operation.state = 'complete'
  AND (
    (receipt.result_kind = 'created' AND EXISTS (
      SELECT 1 FROM legacy_collaboration_comments_v1 comment
      WHERE comment.legacy_comment_id = receipt.legacy_comment_id
        AND comment.last_operation_id = operation.operation_id
    ))
    OR
    (receipt.result_kind = 'deleted' AND NOT EXISTS (
      SELECT 1
      FROM legacy_collaboration_delete_targets_v1 target
      JOIN legacy_collaboration_comments_v1 comment
        ON comment.legacy_comment_id = target.legacy_comment_id
      WHERE target.operation_id = operation.operation_id
    ))
  );
