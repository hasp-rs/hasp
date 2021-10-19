// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use color_eyre::{eyre::WrapErr, Result};
use hasp_metadata::{CargoDirectory, DirectoryHash, DirectoryVersion, PackageDirectory};
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    Connection, Row, ToSql, Transaction,
};

/// Per-directory information stored in the database.
#[derive(Clone, Debug)]
pub(crate) struct DirectoryRow {
    pub(crate) directory_id: i64,
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) hash: DirectoryHash,
    pub(crate) version: DirectoryVersion,
    pub(crate) metadata: CargoDirectory,
}

impl DirectoryRow {
    pub(crate) fn all_matches_for<'a>(
        namespace: &'a str,
        name: &'a str,
        version: &'a DirectoryVersion,
        conn: &'a Connection,
    ) -> Result<Vec<Self>> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT directory_id, namespace, name, hash, version, metadata \
            FROM packages.directories WHERE name == ?1 AND version == ?2",
            )
            .wrap_err_with(|| {
                format!(
                    "error preparing query for {}:{} (version {})",
                    namespace, name, version
                )
            })?;
        let rows = stmt
            .query_and_then([name, version.to_string().as_str()], Self::from_row)
            .wrap_err_with(|| {
                format!(
                    "error querying matches for {}:{} (version {})",
                    namespace, name, version
                )
            })?;

        rows.collect::<rusqlite::Result<Vec<Self>>>()
            .wrap_err_with(|| {
                format!(
                    "error resolving matches for {}:{} version {}",
                    namespace, name, version
                )
            })
    }

    pub(crate) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let directory_id = row.get("directory_id")?;
        let namespace = row.get("namespace")?;
        let name = row.get("name")?;
        let hash = row.get("hash")?;
        let version = row.get("version")?;
        // TODO: match by namespace and use an enum
        let metadata = row.get("metadata")?;

        Ok(Self {
            directory_id,
            namespace,
            name,
            hash,
            version,
            metadata,
        })
    }

    pub(crate) fn to_package_directory(&self) -> PackageDirectory {
        PackageDirectory {
            namespace: self.namespace.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            hash: self.hash,
            metadata: serde_json::to_value(&self.metadata)
                .expect("serialization to value succeeded"),
        }
    }

    pub(crate) fn to_friendly(&self) -> String {
        format!(
            "{}:{} (version {}, hash {})",
            self.namespace, self.name, self.version, self.hash
        )
    }

    /// Gets the most recent state for this row.
    pub(crate) fn get_state(&self, conn: &Connection) -> Result<InstallState> {
        conn.query_row(
            "SELECT state FROM packages.directories WHERE directory_id = ?1",
            [self.directory_id],
            |row| row.get("state"),
        )
        .wrap_err_with(|| format!("failed to get state for {}", self.to_friendly()))
    }

    /// Sets a new state for this row.
    pub(crate) fn set_state(&self, txn: &Transaction, state: InstallState) -> Result<()> {
        txn.execute(
            "UPDATE packages.directories SET state = ?1 WHERE directory_id = ?2",
            params![state, self.directory_id],
        )
        .wrap_err_with(|| format!("failed to set state for {}", self.to_friendly()))?;
        Ok(())
    }
}

// ---
// InstallState
// ---

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstallState {
    NotInstalled,
    Installing,
    Installed,
}

impl FromSql for InstallState {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().and_then(|v| match v {
            "not-installed" => Ok(InstallState::NotInstalled),
            "installing" => Ok(InstallState::Installing),
            "installed" => Ok(InstallState::Installed),
            _other => Err(FromSqlError::InvalidType),
        })
    }
}

impl ToSql for InstallState {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match self {
            InstallState::NotInstalled => Ok("not-installed".into()),
            InstallState::Installing => Ok("installing".into()),
            InstallState::Installed => Ok("installed".into()),
        }
    }
}
