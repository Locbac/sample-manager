-- Deduplicate any existing duplicate assignments
DELETE FROM file_tags WHERE rowid NOT IN (
    SELECT min(rowid) FROM file_tags GROUP BY file_id, tag_id
);

-- Enforce strictly unique tag assignments per file
DROP INDEX IF EXISTS uq_file_tags;
CREATE UNIQUE INDEX uq_file_tags ON file_tags(file_id, tag_id);
