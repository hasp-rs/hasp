// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use color_eyre::{eyre::WrapErr, Result};
use colored::Colorize;
use semver::{BuildMetadata, Op, Version, VersionReq};

/// Split a crate into name and version.
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

pub(crate) fn exact_version_req(req: &VersionReq) -> Option<Version> {
    if req.comparators.len() != 1 {
        return None;
    }
    let comparator = &req.comparators[0];
    if comparator.op != Op::Exact {
        return None;
    }
    // The major, minor and patch versions should all be specified and exact.
    // patch being specified implies minor is specified.
    let major = comparator.major;
    let minor = comparator.minor?;
    let patch = comparator.patch?;
    let pre = comparator.pre.clone();

    Some(Version {
        major,
        minor,
        patch,
        pre,
        // TODO: what about build metadata?
        build: BuildMetadata::EMPTY,
    })
}

#[cfg(test)]
mod tests {
    // TODO: tests for split_version and version_req_is_exact
}
