DELETE FROM messenger_conversations
WHERE ?1 LIKE 'messenger_conversations:%'
  AND id=substr(?1, length('messenger_conversations:') + 1)
  AND EXISTS (
    SELECT 1 FROM business_messenger_legacy_quarantine_v1 item
    WHERE item.source_table='messenger_conversations'
      AND item.source_id=messenger_conversations.id
      AND item.organization_id=?2
      AND item.disposition='quarantined'
      AND item.purge_after_ms<=?3
  )
  AND NOT EXISTS (
    SELECT 1 FROM messenger_messages child
    WHERE child.conversation_id=messenger_conversations.id
  )
  AND NOT EXISTS (
    SELECT 1 FROM messenger_support_emails child
    WHERE child.conversation_id=messenger_conversations.id
  )
