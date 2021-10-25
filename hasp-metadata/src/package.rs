// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{DirectoryVersion, PackageDirectory, ParseHashError};
use camino::Utf8PathBuf;
use chrono::{DateTime, Local};
use once_cell::sync::OnceCell;
use semver::VersionReq;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, fmt, str::FromStr};

// TODO this is incorrect: a string should be parsed as
/// Version requirement.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", transparent)]
pub struct DirectoryVersionReq {
    req: String,
    #[serde(skip)]
    parsed: OnceCell<Option<VersionReq>>,
}

impl DirectoryVersionReq {
    /// Returns the version requirement string.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.req
    }

    /// Returns the version requirement parsed as semver, if successful.
    pub fn as_semver(&self) -> Option<&VersionReq> {
        self.parsed
            .get_or_init(|| self.req.parse::<VersionReq>().ok())
            .as_ref()
    }

    /// Returns true if self matches the version.
    pub fn matches(&self, version: &DirectoryVersion) -> bool {
        match version {
            DirectoryVersion::Semantic(version) => {
                self.as_semver().map_or(false, |req| req.matches(version))
            }
            DirectoryVersion::Literal(version) => &self.req == version,
        }
    }
}

impl fmt::Display for DirectoryVersionReq {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.req)
    }
}

impl From<VersionReq> for DirectoryVersionReq {
    fn from(req: VersionReq) -> Self {
        Self {
            req: format!("{}", req),
            parsed: OnceCell::from(Some(req)),
        }
    }
}

/// Represents a package that is currently installed.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct InstalledPackage {
    /// Information about the package.
    pub package: PackageDirectory,

    /// Information about the installation.
    #[serde(flatten)]
    pub info: InstallInfo,
}

/// Information about an installation. Returned as part of [`InstalledPackage`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct InstallInfo {
    /// Full installation path.
    pub install_path: Utf8PathBuf,

    /// Installation time.
    pub install_time: DateTime<Local>,

    /// A map of installed file names to information about them.
    pub installed_files: BTreeMap<String, InstalledFile>,

    /// Metadata associated with the installation.
    pub metadata: serde_json::Value,
}

/// Represents a binary that is currently installed. Returned as part of [`InstallInfo`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct InstalledFile {
    /// The full path to the installed file.
    pub full_path: Utf8PathBuf,

    /// The hash of the installed file.
    pub hash: FileHash,

    /// Metadata associated with the installed file.
    pub metadata: serde_json::Value,

    /// Whether this file is a binary for which a shim will be created.
    pub is_binary: bool,
}

/// A hash for an installed file. Returned as part of [`InstalledFile`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FileHash {
    Blake3(Blake3Hash),
}

impl fmt::Display for FileHash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FileHash::Blake3(hash) => write!(f, "{}{}", Blake3Hash::PREFIX, hash),
        }
    }
}

impl FromStr for FileHash {
    type Err = ParseHashError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix(Blake3Hash::PREFIX) {
            Some(hash) => hash.parse().map(FileHash::Blake3),
            None => Err(ParseHashError {
                description: "binary hash",
                input: s.into(),
                err: "hash prefix unrecognized".into(),
            }),
        }
    }
}

impl Serialize for FileHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FileHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(D::Error::custom)
    }
}

#[cfg(feature = "rusqlite")]
mod binary_hash_rusqlite_impls {
    use super::*;
    use rusqlite::{
        types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
        ToSql,
    };

    impl FromSql for FileHash {
        fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
            value.as_blob().and_then(|input| {
                // TODO: better error message
                if let Some(bytes) = input.strip_prefix(&Blake3Hash::DB_PREFIX) {
                    return Ok(FileHash::Blake3(Blake3Hash::column_result(
                        ValueRef::Blob(bytes),
                    )?));
                }
                let err =
                    ParseHashError::from_blob("binary hash", input, "hash prefix not recognized");
                Err(FromSqlError::Other(Box::new(err)))
            })
        }
    }

    impl ToSql for FileHash {
        #[inline]
        fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
            let output = match self {
                FileHash::Blake3(hash) => {
                    let mut output = Vec::with_capacity(2 + Blake3Hash::BYTES);
                    output.extend_from_slice(&Blake3Hash::DB_PREFIX);
                    output.extend_from_slice(&hash.to_be_bytes());
                    output
                }
            };
            Ok(output.into())
        }
    }
}

/// A blake3 hash.
#[derive(Clone, Debug)]
pub struct Blake3Hash {
    hash: blake3::Hash,
}

impl Blake3Hash {
    /// The prefix used while serializing a hash.
    pub const PREFIX: &'static str = "blake3:";

    /// The width of this hash, in bytes.
    pub const BYTES: usize = 32;

    /// Creates a new `Blake3Hash` from big-endian bytes.
    #[inline]
    pub fn from_be_bytes(bytes: [u8; Self::BYTES]) -> Self {
        Self { hash: bytes.into() }
    }

    /// Returns a big-endian representation.
    #[inline]
    pub fn to_be_bytes(&self) -> [u8; Self::BYTES] {
        *self.hash.as_bytes()
    }

    const DB_PREFIX: [u8; 2] = *b"01";

    const DESCRIPTION: &'static str = "blake3 hash";
}

impl From<blake3::Hash> for Blake3Hash {
    #[inline]
    fn from(hash: blake3::Hash) -> Self {
        Self { hash }
    }
}

hash_impls!(Blake3Hash, blake3_hash);
