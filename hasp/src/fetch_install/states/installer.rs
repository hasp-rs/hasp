// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    database::DbContext,
    fetch_install::{
        states::helpers::{
            hash_bytes, hash_file, rename_non_racy, ExclusiveRoot, UnlockedRoot, Utf8TempDir,
        },
        PackageMatcher,
    },
    models::directory::DirectoryRow,
};
use async_trait::async_trait;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Local};
use color_eyre::{eyre::WrapErr, Report, Result};
use hasp_metadata::{
    DirectoryHash, DirectoryVersion, FailureReason, InstallFailed, InstallStarted, InstallSuccess,
};
use rusqlite::{named_params, Transaction};
use std::{collections::BTreeMap, fmt, fs, hash::Hasher};
use twox_hash::XxHash64;

#[derive(Debug)]
pub(crate) struct PackageInstaller {
    matcher: PackageMatcher,
    installer: Box<dyn PackageInstallerImpl>,
    temp_dir: Utf8TempDir,
    install_path: Utf8PathBuf,
    row: DirectoryRow,
}

impl PackageInstaller {
    pub(super) fn new(
        matcher: PackageMatcher,
        installer: Box<dyn PackageInstallerImpl>,
        version: DirectoryVersion,
        temp_dir: Utf8TempDir,
    ) -> Result<Self> {
        let mut conn = matcher.db_ctx().creator.create()?;

        // Create a new transaction for the initial lookup since we may end up writing to it.
        let txn = conn.transaction()?;

        let (namespace, name) = (matcher.namespace(), matcher.name());
        // Check if a row exists, and insert it if it doesn't.
        let (install_path, row) = match matcher.best_match_for_version(&version, &txn)? {
            Some(row) => {
                // An existing installation was found.
                let install_path =
                    matcher
                        .hasp_home()
                        .make_install_path(namespace, name, row.package.hash)?;

                // This transaction doesn't do anything further.
                txn.commit()?;

                (install_path, row)
            }
            None => {
                // Generate a new hash and insert the corresponding row.
                let hash = new_directory_hash(namespace, name, &version, &*installer);
                let install_path = matcher
                    .hasp_home()
                    .make_install_path(namespace, name, hash)?;

                let init_context = InitContext {
                    matcher: &matcher,
                    hash,
                    version: &version,
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
            matcher,
            installer,
            temp_dir,
            install_path,
            row,
        })
    }

    pub(crate) async fn install(&self, force: bool) -> Result<InstallStatus> {
        let conn = self.matcher.db_ctx().creator.create()?;

        // Obtain an exclusive lock.
        let lock = self.open_lockfile()?.lock_exclusive()?;
        // What is the current state of the package?
        if self.row.get_installed(&conn)? && !force {
            // The package is already installed.
            Ok(InstallStatus::AlreadyInstalled)
        } else {
            // Start the installation. (The locking means that nothing else would have come
            // along to start the installation.)
            let guard = lock.start_install(true)?;
            match self.install_and_finish(guard).await {
                Ok(()) => Ok(InstallStatus::Success),
                Err(InstallError::Fail(err)) => Ok(InstallStatus::Failure(err)),
                Err(InstallError::Abort(err)) => Err(err),
            }
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
    async fn install_and_finish(&self, mut guard: InstallGuard<'_>) -> Result<(), InstallError> {
        let temp_package = guard.install().await.map_err(|err| {
            err.log_and_rollback(&mut guard);
            err
        })?;

        guard.finish(temp_package).map_err(|err| {
            let err = InstallError::Abort(err);
            err.log_and_rollback(&mut guard);
            err
        })
    }
}

impl AsRef<Utf8Path> for PackageInstaller {
    fn as_ref(&self) -> &Utf8Path {
        &self.install_path
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) enum InstallStatus {
    Success,
    Failure(Report),
    AlreadyInstalled,
}

#[derive(Debug)]
#[must_use]
pub(crate) struct TempInstalledPackage {
    pub(crate) installed_files: BTreeMap<String, TempInstalledFile>,
    pub(crate) metadata: serde_json::Value,
}

#[derive(Debug)]
pub(crate) struct TempInstalledFile {
    pub(crate) temp_path: Utf8PathBuf,
    pub(crate) metadata: serde_json::Value,
    pub(crate) is_binary: bool,
}

#[async_trait]
pub(crate) trait PackageInstallerImpl: fmt::Debug {
    fn installing_metadata(&self) -> serde_json::Value;

    /// Information to add to the directory hash, other than the name and version.
    fn add_to_hasher(&self, hasher: &mut XxHash64);

    /// Installs a package as necessary.
    async fn install(&self) -> Result<TempInstalledPackage>;
}

// ---
// Helpers
// ---

#[derive(Copy, Clone, Debug)]
struct InitContext<'a> {
    matcher: &'a PackageMatcher,
    hash: DirectoryHash,
    version: &'a DirectoryVersion,
    path: &'a Utf8Path,
}

impl<'a> AsRef<Utf8Path> for InitContext<'a> {
    fn as_ref(&self) -> &Utf8Path {
        self.path
    }
}

impl<'a> InitContext<'a> {
    #[inline]
    fn open_lockfile(self) -> Result<UnlockedRoot<Self>> {
        UnlockedRoot::new(self)
    }
}

impl<'a> ExclusiveRoot<InitContext<'a>> {
    /// Inserts a new row, and returns the directory row that was inserted.
    fn insert_new(&self, txn: &Transaction) -> Result<DirectoryRow> {
        // TODO: may actually want to store fetch metadata
        let metadata = self.ctx.matcher.metadata();

        let (namespace, name) = (self.ctx.matcher.namespace(), self.ctx.matcher.name());
        txn.query_row_and_then(
            // Since this can only be inserted while the exclusive lock is held,
            // concurrent connections should never be able to insert the same package.
            // Fail if it happens.
            "INSERT INTO packages.directories (namespace, name, hash, version, metadata, installed) \
                VALUES (:namespace, :name, :hash, :version, :metadata, :installed)\
                RETURNING directory_id, namespace, name, hash, version, metadata",
            named_params! {
                ":namespace": namespace,
                ":name": name,
                ":hash": &self.ctx.hash,
                ":version": &self.ctx.version,
                ":metadata": metadata,
                ":installed": false,
            },
            DirectoryRow::from_row,
        )
        .wrap_err_with(|| {
            format!(
                "failed to insert row for {}:{} (version {}, hash {})",
                namespace, name, self.ctx.version, self.ctx.hash,
            )
        })
    }
}

impl<'inst> ExclusiveRoot<&'inst PackageInstaller> {
    /// Start an installation. Returns an `InstallTransaction`, which is an RAII guard.
    fn start_install(self, force: bool) -> Result<InstallGuard<'inst>> {
        InstallGuard::new(self, force)
    }

    /// Returns the database context (convenience method).
    #[inline]
    fn db_ctx(&self) -> &DbContext {
        self.ctx.matcher.db_ctx()
    }
}

/// Start an installation.
#[derive(Debug)]
#[must_use]
struct InstallGuard<'inst> {
    lock: ExclusiveRoot<&'inst PackageInstaller>,
    force: bool,
    start_time: DateTime<Local>,
    new_dir: Utf8PathBuf,
    old_dir: Utf8PathBuf,
    finished: bool,
}

impl<'inst> InstallGuard<'inst> {
    /// Creates a new install transaction, setting the status in the database to `"installing"`.
    fn new(lock: ExclusiveRoot<&'inst PackageInstaller>, force: bool) -> Result<Self> {
        // Create the old and new directories.
        let new_dir = lock.ctx.temp_dir.path().join("install-new");
        fs::create_dir_all(&new_dir)
            .wrap_err_with(|| format!("failed to create directory at {}", new_dir))?;
        let old_dir = lock.ctx.temp_dir.path().join("install-old");

        let start_time = Local::now();

        // This shouldn't mark the directory as not-installed, because a force-install failure
        // should keep the old version around.

        // This section should never panic.
        {
            let event = InstallStarted {
                package: lock.ctx.row.package.clone(),
                force,
                start_time,
                new_dir: new_dir.clone(),
                old_dir: old_dir.clone(),
            };
            lock.db_ctx().event_logger.log("install_started", &event);
        }

        Ok(Self {
            lock,
            force,
            start_time,
            new_dir,
            old_dir,
            finished: false,
        })
    }

    /// Installs the package into the temp directory.
    async fn install(&self) -> Result<TempInstalledPackage, InstallError> {
        let mut temp_package = self
            .lock
            .ctx
            .installer
            .install()
            .await
            .map_err(InstallError::Fail)?;

        // Move the installed files over to the new directory.
        for (name, installed_file) in &mut temp_package.installed_files {
            let new_path = self.new_dir.join(name);
            std::fs::rename(&installed_file.temp_path, &new_path)
                .wrap_err_with(|| {
                    format!(
                        "failed to rename {} to {}",
                        installed_file.temp_path, new_path
                    )
                })
                .map_err(InstallError::Abort)?;
            installed_file.temp_path = new_path;
        }

        Ok(temp_package)
    }

    /// Commits the install transaction and mark it finished.
    fn finish(&mut self, temp_package: TempInstalledPackage) -> Result<()> {
        if self.finished {
            return Ok(());
        }

        let mut conn = self.lock.db_ctx().creator.create()?;
        let txn = conn.transaction()?;

        // Move the existing install into the old tempdir (if any), and the new install
        // into the new tempdir.
        let install_path = self.lock.ctx.as_ref();
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
                    ":metadata": &temp_package.metadata,
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
        self.row().set_installed(&txn, true)?;

        for (name, installed_file) in &temp_package.installed_files {
            // Hash the file on disk.
            let file_hash = hash_file(&install_path.join(name))?;

            txn.execute(
                "INSERT INTO packages.installed_files (install_id, name, hash, metadata, is_binary)\
                VALUES (:install_id, :name, :hash, :metadata, :is_binary)",
                named_params! {
                    ":install_id": install_id,
                    ":name": name,
                    ":hash": file_hash,
                    ":metadata": &installed_file.metadata,
                    ":is_binary": installed_file.is_binary,
                }
            )
            .wrap_err_with(|| {
                format!(
                    "for {}, failed to insert {} to packages.installed_files",
                    self.row().to_friendly(),
                    name,
                )
            })?;
        }

        txn.commit().wrap_err_with(|| {
            format!(
                "failed to commit transaction for {}",
                self.row().to_friendly(),
            )
        })?;

        // Mark this install as finished at the end. If a rollback is initiated because of a failure
        // in this method, we want it to complete.
        self.finished = true;
        let install_success = InstallSuccess {
            package: self.row().package.clone(),
            force: self.force,
            start_time: self.start_time,
            end_time: Local::now(),
        };
        self.lock
            .db_ctx()
            .event_logger
            .log("install_success", &install_success);

        Ok(())
    }

    /// Explicitly roll back the installation.
    ///
    /// This is called (and errors are ignored) in the `Drop` impl.
    fn rollback(&mut self, reason: FailureReason) -> Result<()> {
        if self.finished {
            return Ok(());
        }

        // Mark this install as finished at the beginning. If a rollback is initiated, but it fails,
        // then we don't want to try and rollback again -- instead, trust that a future process
        // will clean it up.
        self.finished = true;
        let install_failed = InstallFailed {
            package: self.row().package.clone(),
            force: self.force,
            start_time: self.start_time,
            end_time: Local::now(),
            reason,
        };
        self.lock
            .db_ctx()
            .event_logger
            .log("install_failed", &install_failed);

        Ok(())
    }

    // ---
    // Helper methods
    // ---

    #[inline]
    fn row(&self) -> &DirectoryRow {
        &self.lock.ctx.row
    }
}

impl<'inst> Drop for InstallGuard<'inst> {
    fn drop(&mut self) {
        // Ignore errors during rollback.
        let metadata = serde_json::to_value(TRANSACTION_DROPPED)
            .expect("converting a string to a value should never panic");
        let reason = FailureReason::Aborted { metadata };
        let _ = self.rollback(reason);
    }
}

#[derive(Debug)]
enum InstallError {
    Fail(Report),
    Abort(Report),
}

impl InstallError {
    fn log_and_rollback(&self, guard: &mut InstallGuard) {
        // TODO: serialize the error properly
        let reason = match self {
            InstallError::Fail(err) => {
                let metadata = serde_json::to_value(format!("{}", err))
                    .expect("serializing a string should never fail");
                FailureReason::ProcessFailed { metadata }
            }
            InstallError::Abort(err) => {
                let metadata = serde_json::to_value(format!("{}", err))
                    .expect("serializing a string should never fail");
                FailureReason::Aborted { metadata }
            }
        };
        // Ignore errors here.
        let _ = guard.rollback(reason);
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            InstallError::Fail(err) | InstallError::Abort(err) => write!(f, "{}", err),
        }
    }
}

/// Metadata when an install transaction is dropped, likely due to a panic.
///
/// This is returned as the metadata in [`InstallFailureReason::Aborted`].
static TRANSACTION_DROPPED: &str = "transaction dropped, likely due to a panic or Ctrl-C";

fn new_directory_hash(
    namespace: &'static str,
    name: &str,
    version: &DirectoryVersion,
    installer: &(dyn PackageInstallerImpl),
) -> DirectoryHash {
    let mut hasher = XxHash64::default();
    hash_bytes(namespace, &mut hasher);
    hash_bytes(name, &mut hasher);
    hash_bytes(&version.to_string(), &mut hasher);

    installer.add_to_hasher(&mut hasher);

    DirectoryHash::new(hasher.finish())
}
