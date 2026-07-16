UPDATE folders
SET parent_id = ?4,
    depth = COALESCE((SELECT depth + 1 FROM folders WHERE id = ?4), 0),
    revision = revision + 1,
    tree_revision = tree_revision + 1,
    updated_at_ms = ?5,
    last_operation_id = ?6
WHERE id = ?1 AND organization_id = ?2 AND space_id = ?3
