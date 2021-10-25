// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::fetch_install::{
    states::helpers::Utf8TempDir, PackageInstaller, PackageInstallerImpl, PackageMatcher,
};
use async_trait::async_trait;
use camino::Utf8Path;
use color_eyre::{eyre::WrapErr, Result};
use hasp_metadata::DirectoryVersion;
use std::{fmt, fs};

/// Fetches a new package.
#[derive(Debug)]
pub(crate) struct PackageFetcher {
    matcher: PackageMatcher,
    fetcher: Box<dyn PackageFetcherImpl>,
    version: DirectoryVersion,
}

impl PackageFetcher {
    pub(crate) fn new(matcher: PackageMatcher, fetcher: Box<dyn PackageFetcherImpl>) -> Self {
        let version = fetcher.version();

        Self {
            matcher,
            fetcher,
            version,
        }
    }

    pub(crate) async fn fetch(self) -> Result<PackageInstaller> {
        // TODO: consider sharing the fetch dir across installs?

        let cache_dir = self.matcher.hasp_home().cache_dir();
        let temp_dir = Utf8TempDir::new(cache_dir, "install-", "")?;
        let fetch_dir = temp_dir.path().join("fetch");
        fs::create_dir_all(&fetch_dir)
            .wrap_err_with(|| format!("failed to create directory at {}", fetch_dir))?;

        let installer = self
            .fetcher
            .fetch(&fetch_dir)
            .await
            .wrap_err_with(|| format!("failed to fetch package for {}", self.to_friendly()))?;
        PackageInstaller::new(self.matcher, installer, self.version, temp_dir)
    }

    pub(crate) fn to_friendly(&self) -> String {
        format!(
            "{}:{} (version {})",
            self.matcher.namespace(),
            self.matcher.name(),
            self.version
        )
    }
}

/// Represents a way to fetch a specific package.
#[async_trait]
pub(crate) trait PackageFetcherImpl: fmt::Debug {
    /// Returns the version of the package that will be fetched.
    fn version(&self) -> DirectoryVersion;

    /// Fetches the package into the provided directory.
    async fn fetch(&self, fetch_dir: &Utf8Path) -> Result<Box<dyn PackageInstallerImpl>>;
}
