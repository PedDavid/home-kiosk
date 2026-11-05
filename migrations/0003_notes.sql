-- Sticky notes left on the frames or the phone page. to_id is a person id or 'all'. A cleared note
-- keeps its row (cleared = when) so Undo can bring it back; expired or cleared notes aren't served.
CREATE TABLE notes (
  id TEXT PRIMARY KEY,
  text TEXT NOT NULL,
  from_id TEXT NOT NULL,
  to_id TEXT NOT NULL DEFAULT 'all',
  created INTEGER NOT NULL,
  expires INTEGER,
  cleared INTEGER
);
CREATE INDEX notes_live ON notes (cleared, expires);
