-- 0003_musical_keys_enharmonics.sql
-- Add common enharmonic musical keys to ensure 100% lexical match coverage.

INSERT OR IGNORE INTO musical_keys (label, camelot, pc, minor) VALUES
  ('C#m', '12A', 1, 1),
  ('C#',  '3B',  1, 0),
  ('D#m', '2A',  3, 1),
  ('D#',  '5B',  3, 0),
  ('G#m', '1A',  8, 1),
  ('G#',  '4B',  8, 0),
  ('A#m', '3A',  10, 1),
  ('A#',  '6B',  10, 0);
