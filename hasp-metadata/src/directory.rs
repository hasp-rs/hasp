// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{DirectoryHash, DirectoryVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Information about a directory installation for a single package.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct PackageDirectory {
    /// The namespace of the package.
    pub namespace: String,

    /// The name of the package.
    pub name: String,

    /// The version number for this package.
    pub version: DirectoryVersion,

    /// The directory hash for this package.
    pub hash: DirectoryHash,

    /// Additional information, specific to the namespace.
    ///
    /// Based on the namespace, this `Value` can be deserialized into the corresponding metadata.
    pub metadata: Value,
}

/// Specific information associated with Cargo.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct CargoDirectory {
    /// Whether default features were requested.
    pub default_features: bool,
}

json_impls!(CargoDirectory);
