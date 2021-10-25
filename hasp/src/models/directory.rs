// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use color_eyre::{eyre::WrapErr, Result};
use hasp_metadata::{DirectoryVersion, FileHash, PackageDirectory};
use rusqlite::{named_params, params, Connection, Row, Transaction};
use std::collections::BTreeMap;

/// Per-directory information stored in the database.
#[derive(Clone, Debug)]
pub(crate) struct DirectoryRow {
    pub(crate) directory_id: i64,
    pub(crate) package: PackageDirectory,
}

impl DirectoryRow {
    pub(crate) fn all_matches_for(
        namespace: &str,
        name: &str,
        conn: &Connection,
    ) -> Result<Vec<Self>> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT directory_id, namespace, name, hash, version, metadata \
            FROM packages.directories WHERE namespace = ?1 AND name == ?2",
            )
            .wrap_err_with(|| format!("error preparing query for {}:{}", namespace, name))?;
        let rows = stmt
            .query_and_then([namespace, name], Self::from_row)
            .wrap_err_with(|| format!("error querying matches for {}:{}", namespace, name))?;

        rows.collect::<rusqlite::Result<Vec<Self>>>()
            .wrap_err_with(|| format!("error resolving matches for {}:{}", namespace, name))
    }

    pub(crate) fn all_matches_for_version(
        namespace: &str,
        name: &str,
        version: &DirectoryVersion,
        conn: &Connection,
    ) -> Result<Vec<Self>> {
        Self::all_matches_for_version_impl(namespace, name, version, conn).wrap_err_with(|| {
            format!(
                "failed to get known data for {}:{} (version {})",
                namespace, name, version
            )
        })
    }

    fn all_matches_for_version_impl(
        namespace: &str,
        name: &str,
        version: &DirectoryVersion,
        conn: &Connection,
    ) -> Result<Vec<Self>> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT directory_id, namespace, name, hash, version, metadata \
            FROM packages.directories \
            WHERE namespace = :namespace AND name == :name AND version == :version",
            )
            .wrap_err("failed to prepare statement")?;
        let rows = stmt
            .query_and_then(
                named_params! {
                    ":namespace": namespace,
                    ":name": name,
                    ":version": version,
                },
                Self::from_row,
            )
            .wrap_err("failed to query matches")?;

        rows.collect::<rusqlite::Result<Vec<Self>>>()
            .wrap_err("failed to collect matches")
    }

    pub(crate) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let directory_id = row.get("directory_id")?;
        let namespace = row.get("namespace")?;
        let name = row.get("name")?;
        let hash = row.get("hash")?;
        let version = row.get("version")?;
        let metadata = row.get("metadata")?;

        Ok(Self {
            directory_id,
            package: PackageDirectory {
                namespace,
                name,
                hash,
                version,
                metadata,
            },
        })
    }

    pub(crate) fn to_friendly(&self) -> String {
        format!(
            "{}:{} (version {}, hash {})",
            self.package.namespace, self.package.name, self.package.version, self.package.hash
        )
    }

    /// Gets the most recent installed state for this row.
    pub(crate) fn get_installed(&self, conn: &Connection) -> Result<bool> {
        conn.query_row(
            "SELECT installed FROM packages.directories WHERE directory_id = ?1",
            [self.directory_id],
            |row| row.get("installed"),
        )
        .wrap_err_with(|| format!("failed to get installed state for {}", self.to_friendly()))
    }

    /// Sets a new installed state for this row.
    pub(crate) fn set_installed(&self, txn: &Transaction, installed: bool) -> Result<()> {
        txn.execute(
            "UPDATE packages.directories SET installed = ?1 WHERE directory_id = ?2",
            params![installed, self.directory_id],
        )
        .wrap_err_with(|| format!("failed to set installed state for {}", self.to_friendly()))?;
        Ok(())
    }
}

/// Per-install information stored in the database.
#[derive(Clone, Debug)]
pub(crate) struct InstalledRow {
    pub(crate) directory_row: DirectoryRow,
    pub(crate) install_id: i64,
    install_metadata: serde_json::Value,
    binaries: BTreeMap<String, InstalledFileRow>,
}

impl InstalledRow {
    pub(crate) fn all_matches_for(
        namespace: &str,
        name: &str,
        conn: &Connection,
    ) -> Result<Vec<Self>> {
        Self::all_matches_for_impl(namespace, name, conn)
            .wrap_err_with(|| format!("failed to get install data for {}:{}", namespace, name))
    }

    fn all_matches_for_impl(namespace: &str, name: &str, conn: &Connection) -> Result<Vec<Self>> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT \
                    packages.directories.directory_id as directory_id, \
                    namespace, name, hash, version, \
                    packages.directories.metadata as metadata, \
                    install_id, install_time, \
                    packages.installed.metadata as install_metadata \
                FROM packages.directories \
                INNER JOIN packages.installed USING (directory_id) \
                WHERE namespace == :namespace AND name == :name",
            )
            .wrap_err("failed to prepare statement")?;
        let rows = stmt
            .query_and_then(
                named_params! {
                    ":namespace": namespace,
                    ":name": name,
                },
                |row| Self::from_row(conn, row),
            )
            .wrap_err("failed to query rows")?;
        rows.collect::<rusqlite::Result<Vec<Self>>>()
            .wrap_err("failed to collect rows")
    }

    pub(crate) fn from_row(conn: &Connection, row: &Row<'_>) -> rusqlite::Result<Self> {
        let directory_row = DirectoryRow::from_row(row)?;
        let install_id = row.get("install_id")?;
        let install_metadata = row.get("install_metadata")?;

        // Find all the binaries for this install id.
        let binaries = InstalledFileRow::all_matches_for_impl(conn, install_id)?;

        Ok(Self {
            directory_row,
            install_id,
            install_metadata,
            binaries,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InstalledFileRow {
    installed_file_id: i64,
    hash: FileHash,
    file_metadata: serde_json::Value,
    is_binary: bool,
}

impl InstalledFileRow {
    fn all_matches_for_impl(
        conn: &Connection,
        install_id: i64,
    ) -> rusqlite::Result<BTreeMap<String, Self>> {
        let mut stmt = conn.prepare_cached(
            "SELECT installed_file_id, name, hash, metadata, is_binary FROM packages.installed_files \
            WHERE install_id = ?1",
        )?;
        let rows = stmt.query_and_then([install_id], Self::from_row)?;
        rows.collect()
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<(String, Self)> {
        let installed_file_id = row.get("installed_file_id")?;
        let name = row.get("name")?;
        let hash = row.get("hash")?;
        let file_metadata = row.get("metadata")?;
        let is_binary = row.get("is_binary")?;

        Ok((
            name,
            Self {
                installed_file_id,
                hash,
                file_metadata,
                is_binary,
            },
        ))
    }
}
