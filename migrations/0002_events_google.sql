-- Events added on the dashboard are also written to Google Calendar (service account): which
-- calendar, and the event's id there, so removing it on the frame removes it from Google too.
ALTER TABLE events ADD COLUMN gcal TEXT;
ALTER TABLE events ADD COLUMN gid TEXT;
