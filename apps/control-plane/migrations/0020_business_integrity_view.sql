PRAGMA foreign_keys = ON;

-- Expose dirty source findings separately from enforcement so the bounded D1
-- trigger migration remains below the provider compound-expression ceiling.

CREATE VIEW business_source_integrity_v1 AS
WITH RECURSIVE finding_indices(finding_index) AS (
  SELECT 0
  UNION ALL
  SELECT finding_index + 1 FROM finding_indices WHERE finding_index < 6
)
SELECT
  CASE finding_index
    WHEN 0 THEN 'videos_without_scope'
    WHEN 1 THEN 'comments_without_scope'
    WHEN 2 THEN 'video_metadata_without_checksum'
    WHEN 3 THEN 'edit_documents_without_checksum'
    WHEN 4 THEN 'credit_transactions_without_sequence'
    WHEN 5 THEN 'usage_without_operation'
    WHEN 6 THEN 'messenger_quarantined'
  END AS finding,
  CASE finding_index
    WHEN 0 THEN (SELECT COUNT(*) FROM videos WHERE organization_id IS NULL)
    WHEN 1 THEN (SELECT COUNT(*) FROM comments WHERE organization_id IS NULL)
    WHEN 2 THEN (
      SELECT COUNT(*) FROM videos
      WHERE metadata_json IS NOT NULL AND metadata_checksum IS NULL
    )
    WHEN 3 THEN (
      SELECT COUNT(*) FROM video_edits WHERE document_checksum IS NULL
    )
    WHEN 4 THEN (
      SELECT COUNT(*) FROM developer_credit_transactions WHERE ledger_sequence IS NULL
    )
    WHEN 5 THEN (SELECT COUNT(*) FROM usage_ledger WHERE operation_id IS NULL)
    WHEN 6 THEN (SELECT COUNT(*) FROM business_messenger_legacy_quarantine_v1)
  END AS finding_count
FROM finding_indices;
