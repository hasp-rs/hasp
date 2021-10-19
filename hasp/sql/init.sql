-- Create the migrations table.
CREATE TABLE IF NOT EXISTS migration_status (
  migration_id INTEGER PRIMARY KEY,
  -- The name of the migration.
  name TEXT UNIQUE,
  -- The status of the migration (applied or rolled back).
  state TEXT NOT NULL,
  -- The latest time at which a migration was performed.
  apply_time DATETIME,
  -- The latest time at which a migration was rolled back.
  rollback_time DATETIME
);
