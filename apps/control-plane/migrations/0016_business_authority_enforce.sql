PRAGMA foreign_keys = ON;

-- Install the Issue-15 authority and document enforcement after the large
-- expand migration. D1 limits the compound work admitted by one migration;
-- separating these trigger/view statements keeps both halves deterministic
-- without changing their transactional or runtime semantics.

CREATE TRIGGER business_messenger_legacy_quarantine_transition_v1
BEFORE UPDATE ON business_messenger_legacy_quarantine_v1
WHEN NEW.source_table <> OLD.source_table
  OR NEW.source_id <> OLD.source_id
  OR NEW.organization_id IS NOT OLD.organization_id
  OR NEW.quarantined_at_ms <> OLD.quarantined_at_ms
  OR NEW.purge_after_ms <> OLD.purge_after_ms
  OR OLD.disposition <> 'quarantined'
  OR NEW.disposition <> 'purged'
  OR NEW.purged_at_ms < OLD.purge_after_ms
  OR NEW.last_operation_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_quarantine_conflict_v1');
END;

CREATE TRIGGER messenger_conversations_excluded_insert_v1
BEFORE INSERT ON messenger_conversations BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_excluded_fail_closed_v1');
END;
CREATE TRIGGER messenger_conversations_excluded_update_v1
BEFORE UPDATE ON messenger_conversations BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_excluded_fail_closed_v1');
END;
CREATE TRIGGER messenger_messages_excluded_insert_v1
BEFORE INSERT ON messenger_messages BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_excluded_fail_closed_v1');
END;
CREATE TRIGGER messenger_messages_excluded_update_v1
BEFORE UPDATE ON messenger_messages BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_excluded_fail_closed_v1');
END;
CREATE TRIGGER messenger_support_emails_excluded_insert_v1
BEFORE INSERT ON messenger_support_emails BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_excluded_fail_closed_v1');
END;
CREATE TRIGGER messenger_support_emails_excluded_update_v1
BEFORE UPDATE ON messenger_support_emails BEGIN
  SELECT RAISE(ABORT, 'frame_messenger_excluded_fail_closed_v1');
END;
