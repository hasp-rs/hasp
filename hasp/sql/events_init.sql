-- Event table.
CREATE TABLE IF NOT EXISTS journal (
  event_id INTEGER PRIMARY KEY,
  -- Name of the event.
  event_name TEXT NOT NULL,
  -- Timestamp associated with the event (RFC3339).
  event_time DATETIME NOT NULL,
  -- Data associated with this event as a JSON blob.
  data TEXT
);

CREATE INDEX IF NOT EXISTS idx_event_time ON journal (event_time);

-- Assign random values to the event ID to allow concurrent writes (the value is 2**63 - 1).
INSERT OR IGNORE INTO journal (event_id, event_name, event_time) VALUES (9223372036854775807, "sentinel", "now");
