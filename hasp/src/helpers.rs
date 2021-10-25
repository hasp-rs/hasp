// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use color_eyre::{eyre::WrapErr, Result};
use colored::Colorize;
use semver::VersionReq;

/// Split a specifier into name and version.
pub(crate) fn split_version(spec: &str) -> Result<(String, VersionReq)> {
    match spec.split_once('@') {
        Some((name, version)) => {
            let version = version.parse::<VersionReq>().wrap_err_with(|| {
                format!("failed to parse version req for crate {}", name.bold())
            })?;
            Ok((name.to_owned(), version))
        }
        None => Ok((spec.to_owned(), VersionReq::default())),
    }
}

#[cfg(test)]
mod tests {
    // TODO: tests for split_version
}
