DELETE FROM messenger_messages
WHERE ?1 LIKE 'messenger_messages:%'
  AND id=substr(?1, length('messenger_messages:') + 1)
  AND EXISTS (
    SELECT 1 FROM business_messenger_legacy_quarantine_v1 item
    WHERE item.source_table='messenger_messages'
      AND item.source_id=messenger_messages.id
      AND item.organization_id=?2
      AND item.disposition='quarantined'
      AND item.purge_after_ms<=?3
  )
