-- Shopping list. Checked items stay (done = 1) until cleared, so "Got it" can add them back.
CREATE TABLE groceries (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  cat TEXT NOT NULL,
  qty INTEGER NOT NULL DEFAULT 1,
  store TEXT NOT NULL,
  note TEXT NOT NULL DEFAULT '',
  done INTEGER NOT NULL DEFAULT 0,
  cleared INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);

-- One row per chore completed per local day. Chore definitions live in family.json.
CREATE TABLE chore_log (
  chore_id TEXT NOT NULL,
  day TEXT NOT NULL,
  done_by TEXT,
  earner TEXT,
  stars INTEGER NOT NULL DEFAULT 0,
  at INTEGER NOT NULL,
  PRIMARY KEY (chore_id, day)
);
CREATE INDEX chore_log_earner ON chore_log (earner);

-- Events added on the dashboard (the Google calendars are read-only ICS feeds).
CREATE TABLE events (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  people TEXT NOT NULL,
  start INTEGER NOT NULL,
  end INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

-- Photo metadata; the images themselves are in R2 under full/<id>.jpg and small/<id>.jpg.
CREATE TABLE photos (
  id TEXT PRIMARY KEY,
  o TEXT NOT NULL,
  date TEXT NOT NULL,
  place TEXT NOT NULL DEFAULT '',
  from_id TEXT,
  fav INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

-- Expensive upstream results (parsed calendars, forecast) with an expiry, in ms.
CREATE TABLE cache (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  expires INTEGER NOT NULL
);
