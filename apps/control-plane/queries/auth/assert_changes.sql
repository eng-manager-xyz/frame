INSERT INTO auth_repository_assertions_v2(id, satisfied)
VALUES (?1, CASE WHEN changes() = ?2 THEN 1 ELSE 0 END)
