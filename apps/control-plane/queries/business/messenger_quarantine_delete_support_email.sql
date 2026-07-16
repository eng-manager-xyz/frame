DELETE FROM messenger_support_emails
WHERE ?1 LIKE 'messenger_support_emails:%'
  AND id=substr(?1, length('messenger_support_emails:') + 1)
  AND EXISTS (
    SELECT 1 FROM business_messenger_legacy_quarantine_v1 item
    WHERE item.source_table='messenger_support_emails'
      AND item.source_id=messenger_support_emails.id
      AND item.organization_id=?2
      AND item.disposition='quarantined'
      AND item.purge_after_ms<=?3
  )
