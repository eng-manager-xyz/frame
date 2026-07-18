INSERT INTO users(id, email, display_name, created_at_ms, updated_at_ms, status, session_version)
VALUES (?1, ?2, NULL, ?3, ?3, 'active', 0)
