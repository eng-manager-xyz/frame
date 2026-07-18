UPDATE users
SET display_name = CASE WHEN display_name IS NULL THEN ?2 ELSE display_name END,
    email_verified_at_ms = CASE
      WHEN email_verified_at_ms IS NULL THEN ?3 ELSE email_verified_at_ms END,
    updated_at_ms = ?3
WHERE id = ?1
