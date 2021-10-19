// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    cargo_cli::CargoCli,
    crate_info::CrateInfo,
    database::DbContext,
    models::directory::{DirectoryRow, InstallState},
    output::OutputOpts,
};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Local};
use color_eyre::{
    eyre::{bail, WrapErr},
    Report, Result,
};
use fs2::FileExt;
use hasp_metadata::{
    InstallFailed, InstallFailureReason, InstallMethod, InstallStarted, InstallSuccess,
};
use rusqlite::{named_params, params, Connection, Transaction};
use std::{collections::BTreeSet, fs, io};
use tempfile::TempDir;

/// Represents a single installation of a crate.
#[derive(Clone, Debug)]
pub(crate) struct InstallRoot {
    info: CrateInfo,
    install_path: Utf8PathBuf,
    row: DirectoryRow,
    db_ctx: DbContext,
}

impl InstallRoot {
    pub(crate) fn new(info: CrateInfo, hasp_home: &Utf8Path, db_ctx: DbContext) -> Result<Self> {
        let mut conn = db_ctx.creator.create()?;

        // Create a new transaction for the initial lookup since we may end up writing to it.
        let txn = conn.transaction()?;

        // Check if a row exists, and insert it if it doesn't.
        let (install_path, row) = match info.best_match(&txn)? {
            Some(row) => {
                let mut install_path = hasp_home.join("installs");
                install_path.push(&row.namespace);
                install_path.push(row.hash.to_string());
                txn.commit()?;
                (install_path, row)
            }
            None => {
                let hash = info.new_directory_hash();

                let mut install_path = hasp_home.join("installs");
                install_path.push(&info.namespace);
                install_path.push(hash.to_string());

                // Generate a new hash and insert the corresponding row.
                fs::create_dir_all(&install_path)
                    .wrap_err_with(|| format!("failed to create directory at {}", install_path))?;
                let init_context = InitContext {
                    info: &info,
                    path: &install_path,
                };
                let lock = init_context.open_lockfile()?.lock_exclusive()?;
                let row = lock.insert_new(&txn)?;
                // Complete the transaction before releasing the lock to avoid A-B-B-A issues.
                txn.commit()?;
                (install_path, row)
            }
        };

        Ok(Self {
            info,
            install_path,
            row,
            db_ctx,
        })
    }

    #[inline]
    pub(crate) fn row(&self) -> &DirectoryRow {
        &self.row
    }

    /// Returns the full path to the installation directory.
    #[inline]
    pub(crate) fn install_path(&self) -> &Utf8Path {
        &self.install_path
    }

    /// Installs a new package.
    pub(crate) fn install(&self, output_opts: OutputOpts) -> Result<InstallRet> {
        let mut conn = self.db_ctx.creator.create()?;

        // Obtain an exclusive lock.
        let lock = self.open_lockfile()?.lock_exclusive()?;
        // What is the current state of the package?
        let txn = conn.transaction()?;
        let state = self.row.get_state(&txn)?;
        match state {
            InstallState::NotInstalled => {
                // Mark the crate as being installed. (The locking means that nothing else would
                // have come along to update this process.)
                let guard = lock.start_install(txn, false)?;
                self.install_impl(guard, output_opts)
            }
            InstallState::Installing => {
                // TODO: means the install process died -- need to clean up
                log::info!("cleaning up aborted install for {}", self.info.name);
                todo!("need to implement aborted install cleanup")
            }
            InstallState::Installed => Ok(InstallRet::AlreadyInstalled),
        }
    }

    // ---
    // Helper methods
    // ---

    #[inline]
    fn open_lockfile(&self) -> Result<UnlockedRoot<&'_ Self>> {
        UnlockedRoot::new(self)
    }

    #[inline]
    fn install_impl(
        &self,
        mut guard: InstallGuard<'_>,
        output_opts: OutputOpts,
    ) -> Result<InstallRet> {
        let error_handler = |err: Report, guard: &mut InstallGuard| {
            // TODO: serialize the error
            let metadata = serde_json::to_value(format!("{}", err))
                .expect("serializing a string should never fail");
            let reason = InstallFailureReason::Aborted { metadata };
            // Ignore errors here.
            let _ = guard.rollback(reason);
            err
        };

        let ret = guard
            .install(output_opts)
            .wrap_err_with(|| {
                format!(
                    "failed to install {} at {}",
                    self.row.to_friendly(),
                    self.install_path
                )
            })
            .map_err(|err| error_handler(err, &mut guard))?;

        match ret {
            InstallAttempted::Success => {
                guard
                    .finish()
                    .map_err(|err| error_handler(err, &mut guard))?;
            }
            InstallAttempted::Failure => {
                // TODO: get more structured failure metadata
                let metadata = serde_json::to_value("install failed")
                    .expect("serializing string should always work");
                let reason = InstallFailureReason::ProcessFailed { metadata };
                guard
                    .rollback(reason)
                    .map_err(|err| error_handler(err, &mut guard))?;
            }
        }

        Ok(InstallRet::Attempted(ret))
    }
}

impl AsRef<Utf8Path> for InstallRoot {
    fn as_ref(&self) -> &Utf8Path {
        &self.install_path
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstallRet {
    Attempted(InstallAttempted),
    AlreadyInstalled,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstallAttempted {
    Success,
    Failure,
}

const LOCKFILE_EXT: &str = "lock";

#[derive(Copy, Clone, Debug)]
struct InitContext<'root> {
    info: &'root CrateInfo,
    path: &'root Utf8Path,
}

impl<'root> AsRef<Utf8Path> for InitContext<'root> {
    fn as_ref(&self) -> &Utf8Path {
        self.path
    }
}

impl<'root> InitContext<'root> {
    #[inline]
    fn open_lockfile(self) -> Result<UnlockedRoot<Self>> {
        UnlockedRoot::new(self)
    }
}

#[derive(Debug)]
pub(crate) struct UnlockedRoot<T> {
    file: fs::File,
    lock_path: Utf8PathBuf,
    ctx: T,
}

impl<T: AsRef<Utf8Path>> UnlockedRoot<T> {
    fn new(ctx: T) -> Result<Self> {
        let mut lock_path = ctx.as_ref().to_path_buf();
        // Create the lockfile in the same directory as the install, so that old and new
        // directories can be atomically swapped in.
        lock_path.set_extension(LOCKFILE_EXT);
        let mut open_opts = fs::OpenOptions::new();
        // Create the file if it doesn't exist.
        let file = open_opts
            .write(true)
            .create(true)
            .open(&lock_path)
            .wrap_err_with(|| format!("failed to open install lock at {}", lock_path))?;
        Ok(Self {
            file,
            lock_path,
            ctx,
        })
    }

    #[inline]
    fn lock_exclusive(self) -> Result<ExclusiveRoot<T>> {
        self.file
            .lock_exclusive()
            .wrap_err_with(|| format!("failed to obtain exclusive lock at {}", self.lock_path))?;
        Ok(ExclusiveRoot {
            file: self.file,
            ctx: self.ctx,
        })
    }

    #[inline]
    #[allow(dead_code)]
    fn lock_shared(self) -> Result<SharedRoot<T>> {
        self.file
            .lock_shared()
            .wrap_err_with(|| format!("failed to obtain shared lock at {}", self.lock_path))?;
        Ok(SharedRoot {
            file: self.file,
            ctx: self.ctx,
        })
    }
}

/// Operations that can only be performed on a root where the shared lock has been acquired.
#[derive(Debug)]
#[must_use]
pub(crate) struct SharedRoot<T> {
    file: fs::File,
    ctx: T,
}

/// Operations that can only be performed on a root where the exclusive lock has been acquired.
/// This forms a superset of the operations on the shared root.
#[derive(Debug)]
#[must_use]
pub(crate) struct ExclusiveRoot<T> {
    file: fs::File,
    ctx: T,
}

impl<'root> ExclusiveRoot<InitContext<'root>> {
    /// Inserts a new row, and returns the directory row that was inserted.
    fn insert_new(&self, txn: &Transaction) -> Result<DirectoryRow> {
        let metadata = self.ctx.info.to_metadata();
        let hash = self.ctx.info.new_directory_hash();

        txn.query_row_and_then(
            // Since this can only be inserted while the exclusive lock is held,
            // concurrent connections should never be able to insert the same package.
            // Fail if it happens.
            "INSERT INTO packages.directories (namespace, name, hash, version, metadata, state) \
                VALUES (:namespace, :name, :hash, :version, :metadata, :state)\
                RETURNING directory_id, namespace, name, hash, version, metadata",
            named_params! {
                ":namespace": self.ctx.info.namespace,
                ":name": self.ctx.info.name,
                ":hash": &hash,
                ":version": format!("{}", self.ctx.info.version),
                ":metadata": metadata,
                ":state": InstallState::NotInstalled,
            },
            DirectoryRow::from_row,
        )
        .wrap_err_with(|| {
            format!(
                "failed to insert row for {}:{} (version {}, hash {})",
                self.ctx.info.namespace, self.ctx.info.name, self.ctx.info.version, hash,
            )
        })
    }
}

impl<'root> ExclusiveRoot<&'root InstallRoot> {
    /// Start an installation. Returns an `InstallTransaction`, which is an RAII guard.
    fn start_install(self, txn: Transaction, force: bool) -> Result<InstallGuard<'root>> {
        InstallGuard::new(self, txn, force)
    }
}

/// Start an installation.
#[derive(Debug)]
#[must_use]
struct InstallGuard<'root> {
    lock: ExclusiveRoot<&'root InstallRoot>,
    installing_id: i64,
    force: bool,
    start_time: DateTime<Local>,
    new_tempdir: TempDir,
    new_dir: Utf8PathBuf,
    old_tempdir: TempDir,
    old_dir: Utf8PathBuf,
    finished: bool,
}

impl<'root> InstallGuard<'root> {
    /// Creates a new install transaction, setting the status in the database to `"installing"`.
    fn new(lock: ExclusiveRoot<&'root InstallRoot>, txn: Transaction, force: bool) -> Result<Self> {
        // Create a new temporary directory that will hold the path.
        let (new_tempdir, new_dir) = new_tempdir(lock.ctx.install_path())?;
        let (old_tempdir, old_dir) = old_tempdir(lock.ctx.install_path())?;

        lock.ctx.row().set_state(&txn, InstallState::Installing)?;

        // TODO: other install methods
        let method = InstallMethod::CARGO_LOCAL;
        let start_time = Local::now();

        // TODO: cargo-specific metadata
        let installing_id = txn
            .query_row(
                "INSERT INTO packages.installing \
        (directory_id, install_method, force, start_time, new_dir, old_dir, metadata) \
        VALUES (:directory_id, :install_method, :force, :start_time, :new_dir, :old_dir, :metadata) \
        RETURNING installing_id",
                named_params! {
                    ":directory_id": lock.ctx.row.directory_id,
                    ":install_method": &method,
                    ":force": force,
                    ":start_time": &start_time,
                    ":new_dir": new_dir.as_str(),
                    ":old_dir": old_dir.as_str(),
                    ":metadata": serde_json::Value::Null,
                },
                |row| row.get("installing_id"),
            )
            .wrap_err_with(|| {
                format!(
                    "failed to insert installing information into DB for {}",
                    lock.ctx.row.to_friendly()
                )
            })?;

        // This section should never panic.
        {
            let event = InstallStarted {
                package: lock.ctx.row.to_package_directory(),
                method,
                force,
                start_time,
                new_dir: new_dir.clone(),
                old_dir: old_dir.clone(),
            };
            lock.ctx.db_ctx.event_logger.log("install_started", &event);
        }

        txn.commit()?;

        Ok(Self {
            lock,
            installing_id,
            force,
            start_time,
            new_tempdir,
            new_dir,
            old_tempdir,
            old_dir,
            finished: false,
        })
    }

    /// Installs the package into the temp directory.
    fn install(&self, output_opts: OutputOpts) -> Result<InstallAttempted> {
        // TODO: fetch from other sources, better error handling, etc etc
        let mut cargo_cli = CargoCli::new("install", output_opts);
        let version_str = format!(
            "={}",
            self.row()
                .version
                .as_semantic()
                .expect("cargo versions should be semantic")
        );
        cargo_cli.add_args([
            &self.row().name,
            "--vers",
            version_str.as_str(),
            "--root",
            self.new_dir.as_str(),
            // TODO: frozen/locked etc
        ]);

        let output = cargo_cli
            .to_expression()
            .unchecked()
            .run()
            .wrap_err("failed to run `cargo install`")?;
        if output.status.success() {
            Ok(InstallAttempted::Success)
        } else {
            Ok(InstallAttempted::Failure)
        }
    }

    /// Commits the install transaction and mark it finished.
    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }

        let mut conn = self.lock.ctx.db_ctx.creator.create()?;
        let txn = conn.transaction()?;
        self.delete_installing_row(&txn)?;

        // Move the existing install into the old tempdir (if any), and the new install
        // into the new tempdir.
        let install_path = self.lock.ctx.install_path();
        rename_non_racy(install_path, &self.old_dir)?;
        fs::rename(&self.new_dir, install_path).wrap_err_with(|| {
            format!(
                "failed to rename new directory {} to install path {}",
                &self.new_dir, install_path
            )
        })?;

        // Add the install to packages.installed.
        let install_time = Local::now();
        let install_id: i64 = txn
            .query_row(
                "INSERT INTO packages.installed (directory_id, install_time, metadata)\
        VALUES (:directory_id, :install_time, :metadata)\
        RETURNING install_id",
                named_params! {
                    ":directory_id": self.row().directory_id,
                    ":install_time": install_time,
                    // TODO metadata
                    ":metadata": serde_json::Value::Null,
                },
                |row| row.get("install_id"),
            )
            .wrap_err_with(|| {
                format!(
                    "failed to add {} to packages.installed",
                    self.row().to_friendly()
                )
            })?;

        // Update the state to installed.
        self.row().set_state(&txn, InstallState::Installed)?;

        // List out all the binaries installed by iterating through the directory.
        // TODO: stop relying on cargo install and use artifact messages instead.
        let binaries = list_binaries(install_path)
            .wrap_err_with(|| format!("failed to list binaries for {}", install_path))?;

        // Add binaries to packages.binaries.
        for binary in &binaries {
            txn.execute(
                "INSERT INTO packages.binaries (name, install_id) VALUES (?1, ?2)",
                params![binary, install_id],
            )
            .wrap_err_with(|| {
                format!(
                    "for {}, failed to insert binary {} to packages.binaries",
                    self.row().to_friendly(),
                    binary
                )
            })?;
        }

        txn.commit().wrap_err_with(|| {
            format!(
                "for {}, failed to commit transaction for installing_id {}",
                self.row().to_friendly(),
                self.installing_id
            )
        })?;

        // Mark this install as finished at the end. If a rollback is initiated because of a failure
        // in this method, we want it to complete.
        self.finished = true;
        let install_success = InstallSuccess {
            package: self.row().to_package_directory(),
            // TODO: other install methods
            method: InstallMethod::CARGO_LOCAL,
            force: self.force,
            start_time: self.start_time,
            end_time: Local::now(),
        };
        self.lock
            .ctx
            .db_ctx
            .event_logger
            .log("install_success", &install_success);

        Ok(())
    }

    /// Explicitly roll back the installation.
    ///
    /// This is called (and errors are ignored) in the `Drop` impl.
    fn rollback(&mut self, reason: InstallFailureReason) -> Result<()> {
        if self.finished {
            return Ok(());
        }

        // Mark this install as finished at the beginning. If a rollback is initiated, but it fails,
        // then we don't want to try and rollback again -- instead, trust that a future process
        // will clean it up.
        self.finished = true;
        let install_failed = InstallFailed {
            package: self.row().to_package_directory(),
            // TODO: other install methods
            method: InstallMethod::CARGO_LOCAL,
            force: self.force,
            start_time: self.start_time,
            end_time: Local::now(),
            reason,
        };
        self.lock
            .ctx
            .db_ctx
            .event_logger
            .log("install_failed", &install_failed);

        // Clean up the database.
        let mut conn = self.lock.ctx.db_ctx.creator.create()?;
        let txn = conn.transaction()?;
        self.delete_installing_row(&txn)?;
        self.row().set_state(&txn, InstallState::NotInstalled)?;

        txn.commit().wrap_err_with(|| {
            format!(
                "for {}, failed to commit rollback for installing_id {}",
                self.row().to_friendly(),
                self.installing_id
            )
        })?;
        Ok(())
    }

    // ---
    // Helper methods
    // ---

    fn delete_installing_row(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "DELETE FROM packages.installing WHERE installing_id = ?1",
            [self.installing_id],
        )
        .wrap_err_with(|| {
            format!(
                "failed to delete installing_id {} for {}",
                self.installing_id,
                self.row().to_friendly()
            )
        })?;
        Ok(())
    }

    #[inline]
    fn row(&self) -> &DirectoryRow {
        self.lock.ctx.row()
    }
}

impl<'root> Drop for InstallGuard<'root> {
    fn drop(&mut self) {
        // Ignore errors during rollback.
        let metadata = serde_json::to_value(TRANSACTION_DROPPED)
            .expect("converting a string to a value should never panic");
        let reason = InstallFailureReason::Aborted { metadata };
        let _ = self.rollback(reason);
    }
}

/// Metadata when an install transaction is dropped, likely due to a panic.
///
/// This is returned as the metadata in [`InstallFailureReason::Aborted`].
static TRANSACTION_DROPPED: &str = "transaction dropped, likely due to a panic or Ctrl-C";

// ---
// Helper functions
// ---

fn new_tempdir(install_path: &Utf8Path) -> Result<(TempDir, Utf8PathBuf)> {
    tempdir_impl(install_path, ".new")
}

fn old_tempdir(install_path: &Utf8Path) -> Result<(TempDir, Utf8PathBuf)> {
    tempdir_impl(install_path, ".old")
}

fn tempdir_impl(install_path: &Utf8Path, suffix: &str) -> Result<(TempDir, Utf8PathBuf)> {
    // Create the temp dir in the same directory as the install path, so it can be swapped
    // in and out atomically.
    let parent = install_path.parent().expect("install path has a parent");
    let prefix = install_path
        .file_name()
        .expect("install path has a last component");

    let mut builder = tempfile::Builder::new();
    let tempdir = builder
        .prefix(prefix)
        .suffix(suffix)
        .tempdir_in(parent)
        .wrap_err_with(|| format!("error creating temporary directory in {}", parent))?;

    let path = Utf8Path::from_path(tempdir.path())
        .expect("tempdir should be UTF-8")
        .to_path_buf();
    Ok((tempdir, path))
}

/// Rename a directory to another, ignoring file not found issues.
fn rename_non_racy(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(err) => {
            if err.kind() == io::ErrorKind::NotFound {
                // Skip this -- means src doesn't exist.
                Ok(())
            } else {
                bail!("failed to rename existing directory {} to {}", src, dest)
            }
        }
    }
}

fn list_binaries(install_path: &Utf8Path) -> Result<BTreeSet<String>> {
    let mut binaries = BTreeSet::new();
    // This is tied to cargo's implementation details.
    // TODO: skipping cargo install will fix this -- really, this whole function should
    // be thrown away.
    let bin_dir = install_path.join("bin");
    for entry in bin_dir.read_dir()? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let file_name = match entry.file_name().into_string() {
            Ok(file_name) => file_name,
            Err(original) => bail!(
                "in install path {}, entry {} is not valid UTF-8",
                install_path,
                original.to_string_lossy()
            ),
        };
        if file_type.is_file() {
            binaries.insert(file_name);
        }
    }
    Ok(binaries)
}
