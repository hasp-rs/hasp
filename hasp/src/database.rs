// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::events::EventLogger;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Local;
use color_eyre::{
    eyre::{bail, WrapErr},
    Report, Result,
};
use include_dir::{include_dir, Dir};
use once_cell::sync::OnceCell;
use rusqlite::{params, Connection, DatabaseName, Transaction};
use serde::Serialize;
use std::{collections::BTreeMap, fmt, sync::Arc};

const SQL_DIR: Dir = include_dir!("sql");

#[derive(Clone, Debug)]
pub(crate) struct DbContext {
    pub(crate) creator: ConnectionCreator,
    pub(crate) event_logger: EventLogger,
}

#[derive(Clone, Debug)]
pub(crate) struct ConnectionCreator {
    inner: Arc<dyn CreateConnectionImpl>,
    initialized: Arc<OnceCell<()>>,
}

impl ConnectionCreator {
    const DATABASES: [DatabaseName<'static>; 2] =
        [DatabaseName::Main, DatabaseName::Attached("packages")];

    // SQLite application id: hex representation of the string "hasp".
    const APPLICATION_ID_PRAGMA: &'static str = "application_id";
    const APPLICATION_ID: i32 = 0x68617370;

    // Busy timeout.
    const BUSY_TIMEOUT_MS: u32 = 5000;

    pub(crate) fn new(hasp_home: impl Into<Utf8PathBuf>) -> Self {
        Self {
            inner: Arc::new(DiskDb {
                hasp_home: hasp_home.into(),
            }),
            initialized: Arc::new(OnceCell::new()),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn new_in_memory() -> Self {
        Self {
            inner: Arc::new(InMemoryDb),
            initialized: Arc::new(OnceCell::new()),
        }
    }

    pub(crate) fn create(&self) -> Result<Connection> {
        let conn = self.inner.create_impl()?;

        // Turn on foreign key support and a busy timeout.
        conn.pragma_update(None, "foreign_keys", "ON")
            .wrap_err_with(|| {
                format!(
                    "enabling foreign keys failed for {}",
                    self.inner.description()
                )
            })?;
        conn.pragma_update(None, "busy_timeout", Self::BUSY_TIMEOUT_MS)
            .wrap_err_with(|| {
                format!(
                    "setting busy timeout to {} ms failed for {}",
                    Self::BUSY_TIMEOUT_MS,
                    self.inner.description()
                )
            })?;

        Ok(conn)
    }

    pub(crate) fn create_events(&self) -> Result<Connection> {
        let conn = self.inner.create_events()?;

        // Turn on the busy timeout (foreign key support isn't required).
        conn.pragma_update(None, "busy_timeout", Self::BUSY_TIMEOUT_MS)
            .wrap_err_with(|| {
                format!(
                    "setting busy timeout to {} ms failed for {}",
                    Self::BUSY_TIMEOUT_MS,
                    self.inner.description()
                )
            })?;

        Ok(conn)
    }

    /// Create a connection and initialize it.
    pub(crate) fn initialize(&self, event_logger: &EventLogger) -> Result<()> {
        let mut conn = self.create()?;
        let events_conn = self.create_events()?;

        // Initialize and run migrations the first time this creator opens a connection.
        // TODO: should we open the db in read-only mode sometimes?
        self.initialized.get_or_try_init::<_, Report>(|| {
            for db in Self::DATABASES {
                // Turn on WAL -- this is persistent.
                // (Must be done outside of a transaction.)
                self.enable_wal(&conn, db)?;
            }
            self.enable_wal(&events_conn, DatabaseName::Main)?;

            let txn = conn.transaction()?;

            for db in Self::DATABASES {
                // Write out the application ID -- this is persistent.
                // TODO: read it to check its value and fail if it doesn't match?
                self.set_application_id(&txn, db)
                    .wrap_err("setting application ID failed")?;
            }

            self.set_application_id(&events_conn, DatabaseName::Main)
                .wrap_err("setting application ID failed for events DB")?;

            // Initialize tables that stay the same.

            let init_sql = SQL_DIR
                .get_file("init.sql")
                .expect("sql/init.sql exists")
                .contents_utf8()
                .expect("sql/init.sql is valid UTF-8");
            txn.execute_batch(init_sql).wrap_err_with(|| {
                format!(
                    "creating initial tables failed for {}",
                    self.inner.description()
                )
            })?;

            let events_init_sql = SQL_DIR
                .get_file("events_init.sql")
                .expect("sql/events_init.sql exists")
                .contents_utf8()
                .expect("sql/events_init.sql is valid UTF-8");
            events_conn
                .execute_batch(events_init_sql)
                .wrap_err_with(|| {
                    format!(
                        "initializing events db failed at {}",
                        self.inner.description()
                    )
                })?;

            // Run migrations.
            run_migrations(&txn, event_logger).wrap_err_with(|| {
                format!("running migrations failed for {}", self.inner.description())
            })?;

            txn.commit().wrap_err_with(|| {
                format!(
                    "committing initial transaction failed for {}",
                    self.inner.description()
                )
            })
        })?;

        Ok(())
    }

    // ---
    // Helper methods
    // ---

    fn enable_wal(&self, conn: &Connection, db: DatabaseName) -> Result<()> {
        conn.pragma_update(Some(db), "journal_mode", "WAL")
            .wrap_err_with(|| {
                format!(
                    "turning on write-ahead logging failed for {} (database {:?})",
                    self.inner.description(),
                    db
                )
            })
    }

    fn set_application_id(&self, conn: &Connection, db: DatabaseName) -> Result<()> {
        let mut application_id = 0;

        conn.pragma_query(Some(db), Self::APPLICATION_ID_PRAGMA, |row| {
            application_id = row.get(0)?;
            Ok(())
        })
        .wrap_err_with(|| {
            format!(
                "query application ID failed for {} (database {:?})",
                self.inner.description(),
                db
            )
        })?;
        if application_id != Self::APPLICATION_ID {
            conn.pragma_update(Some(db), "application_id", Self::APPLICATION_ID)
                .wrap_err_with(|| {
                    format!(
                        "setting application ID failed for {} (database {:?})",
                        self.inner.description(),
                        db
                    )
                })?;
        }
        Ok(())
    }
}

fn run_migrations(txn: &Transaction, event_logger: &EventLogger) -> Result<()> {
    let all_migrations: BTreeMap<&'static str, &'static str> = SQL_DIR
        .get_dir("migrations")
        .expect("migrations should exist")
        .dirs()
        .iter()
        .map(|dir| {
            // Construct a map by file names.
            let migration_name = Utf8Path::from_path(dir.path())
                .expect("migrations are UTF-8")
                .file_name()
                .expect("directory names present");
            let file_path = dir.path().join("up.sql");
            let sql = dir
                .get_file(&file_path)
                .unwrap_or_else(|| panic!("{} does not exist", file_path.display()))
                .contents_utf8()
                .expect("up.sql is valid UTF-8");
            (migration_name, sql)
        })
        .collect();

    // Look for all migrations that haven't been run yet.
    let mut stmt = txn.prepare(
        r#"SELECT name, state, apply_time FROM migration_status
        WHERE state == "applied"
        ORDER BY name DESC"#,
    )?;
    let mut rows = stmt.query([])?;
    let last_applied: Option<String> = rows
        .next()?
        .map(|row| row.get("name").expect("name field is text"));

    let migrations_to_perform = match &last_applied {
        Some(last_applied) => {
            // Need this dance to ensure we get the static reference to last_applied from the
            // btreemap.
            let last_applied_key = all_migrations.get_key_value(last_applied.as_str());
            match last_applied_key {
                Some((&last_applied, _)) => {
                    let mut ret = all_migrations.range(last_applied..);
                    // migrations to perform are whatever comes *after* ret.
                    ret.next();
                    ret
                }
                None => {
                    let last_known = all_migrations
                        .keys()
                        .last()
                        .expect("at least one migration known to hasp");
                    bail!(
                        "latest applied migration {} is newer than latest known migration {}\
                        (hint: upgrade hasp version)",
                        last_applied,
                        last_known,
                    );
                }
            }
        }
        None => {
            // No migrations found -- apply all of them.
            all_migrations.range::<&str, _>(..)
        }
    };

    // Perform all the migrations one by one.
    let mut migrations_performed = vec![];
    if migrations_performed.is_empty() {}

    for (&name, sql) in migrations_to_perform {
        let data = MigrationData { name };

        tracing::debug!("running migration {}", name);
        event_logger.log("migration_started", &data);
        match run_one_migration(txn, name, sql) {
            Ok(()) => {
                migrations_performed.push(name);
                event_logger.log("migration_finished", &data);
            }
            Err(err) => {
                let rollback_data = RollbackData {
                    rolled_back: migrations_performed,
                };
                event_logger.log("migration_rollback", &rollback_data);
                return Err(err);
            }
        }
    }

    Ok(())
}

fn run_one_migration(txn: &Transaction, name: &'static str, sql: &str) -> Result<()> {
    txn.execute_batch(sql)
        .wrap_err_with(|| format!("failed to perform migration {}", name))?;
    txn.execute(
        "INSERT INTO migration_status (name, state, apply_time) VALUES (?1, ?2, ?3)",
        params![name, "applied", Local::now()],
    )
    .wrap_err_with(|| format!("failed to insert migration {} into table", name))?;
    Ok(())
}

#[derive(Debug, Serialize)]
struct MigrationData {
    name: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct RollbackData {
    rolled_back: Vec<&'static str>,
}

// ---
// Database backend
// ---

trait CreateConnectionImpl: Sync + Send + fmt::Debug {
    /// Backend implementation that creates the database and attaches packages and events instances
    /// to it.
    fn create_impl(&self) -> Result<Connection>;
    fn create_events(&self) -> Result<Connection>;
    fn description(&self) -> &str;
}

// ---
// Database backend implementations
// ---

#[derive(Clone, Debug)]
pub(crate) struct DiskDb {
    hasp_home: Utf8PathBuf,
}

impl CreateConnectionImpl for DiskDb {
    fn create_impl(&self) -> Result<Connection> {
        let db = self.hasp_home.join("db.sqlite");
        let packages = self.hasp_home.join("packages.sqlite");

        let conn =
            Connection::open(&db).wrap_err_with(|| format!("opening DB at {} failed", db))?;

        // Attach the packages DB.
        conn.execute("ATTACH DATABASE ?1 as packages", [packages.as_str()])
            .wrap_err_with(|| format!("attaching packages DB at {} failed", packages))?;

        Ok(conn)
    }

    fn create_events(&self) -> Result<Connection> {
        let events = self.hasp_home.join("events.sqlite");
        Connection::open(&events)
            .wrap_err_with(|| format!("opening events DB at {} failed", events))
    }

    fn description(&self) -> &str {
        self.hasp_home.as_str()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InMemoryDb;

impl CreateConnectionImpl for InMemoryDb {
    fn create_impl(&self) -> Result<Connection> {
        let conn = Connection::open_in_memory().wrap_err("opening in memory db failed")?;
        conn.execute("ATTACH DATABASE ?1 as packages", [":memory:"])
            .wrap_err("attaching in-memory packages DB failed")?;

        Ok(conn)
    }

    fn create_events(&self) -> Result<Connection> {
        Connection::open_in_memory().wrap_err("opening in-memory events DB failed")
    }

    fn description(&self) -> &str {
        "in-memory database"
    }
}
