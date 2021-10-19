// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::models::directory::DirectoryRow;
use color_eyre::Result;
use hasp_metadata::{CargoDirectory, DirectoryHash, DirectoryVersion};
use rusqlite::Connection;
use std::hash::Hasher;
use twox_hash::XxHash64;

/// Information about a specific crate -- used to fetch crates by hash etc.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CrateInfo {
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) version: DirectoryVersion,
    pub(crate) default_features: bool,
    // TODO: features, registry, git etc
}

impl CrateInfo {
    pub(crate) fn best_match(&self, conn: &Connection) -> Result<Option<DirectoryRow>> {
        let rows = DirectoryRow::all_matches_for(&self.namespace, &self.name, &self.version, conn)?;
        Ok(rows.into_iter().next())
    }

    /// Returns crate metadata (everything other than the name and version) as stored in sqlite.
    pub(crate) fn to_metadata(&self) -> CargoDirectory {
        CargoDirectory {
            default_features: self.default_features,
        }
    }

    /// Create a new directory hash from a `CrateInfo`.
    ///
    /// This hash should not be used for initial lookups, as it can change over time.
    pub(crate) fn new_directory_hash(&self) -> DirectoryHash {
        let mut hasher = XxHash64::default();
        hash_bytes(&self.namespace, &mut hasher);
        hash_bytes(&self.name, &mut hasher);
        hash_bytes(self.version.to_string(), &mut hasher);

        // TODO: features, registry, git etc

        DirectoryHash::new(hasher.finish())
    }
}

fn hash_bytes(bytes: impl AsRef<[u8]>, hasher: &mut XxHash64) {
    let bytes = bytes.as_ref();
    // This is similar to https://doc.rust-lang.org/beta/nightly-rustc/rustc_data_structures/stable_hasher/trait.HashStable.html.
    hasher.write_u64(bytes.len() as u64);
    hasher.write(bytes);
}
