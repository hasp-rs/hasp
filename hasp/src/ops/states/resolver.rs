// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    ops::{PackageFetcher, PackageFetcherImpl, PackageMatcher},
    output::OutputOpts,
};
use async_trait::async_trait;
use color_eyre::{eyre::WrapErr, Result};
use hasp_metadata::DirectoryVersionReq;
use std::fmt;

/// Resolves a version requirement into a specific version.
#[derive(Debug)]
pub struct PackageResolver {
    matcher: PackageMatcher,
    resolver: Box<dyn PackageResolverImpl>,
}

impl PackageResolver {
    pub(crate) fn new(matcher: PackageMatcher, resolver: Box<dyn PackageResolverImpl>) -> Self {
        Self { matcher, resolver }
    }

    #[inline]
    pub(crate) async fn make_fetcher(self) -> Result<PackageFetcher> {
        let fetcher = self
            .resolver
            .resolve(
                self.matcher.name().to_owned(),
                self.matcher.req().clone(),
                self.matcher.output_opts(),
            )
            .await
            .wrap_err_with(|| {
                format!(
                    "failed to create fetcher for {}",
                    self.matcher.to_friendly()
                )
            })?;

        Ok(PackageFetcher::new(self.matcher, fetcher))
    }
}

/// Represents a way to match a specific package.
#[async_trait]
pub(crate) trait PackageResolverImpl: fmt::Debug {
    /// Resolves this package into a specific version, and returns a fetcher.
    async fn resolve(
        &self,
        name: String,
        req: DirectoryVersionReq,
        output_opts: OutputOpts,
    ) -> Result<Box<dyn PackageFetcherImpl>>;
}
