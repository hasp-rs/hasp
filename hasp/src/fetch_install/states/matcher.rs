// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    database::DbContext,
    fetch_install::{PackageResolver, PackageResolverImpl},
    models::directory::{DirectoryRow, InstalledRow},
    output::OutputOpts,
    state::HaspHome,
};
use async_trait::async_trait;

use color_eyre::{eyre::WrapErr, Result};
use hasp_metadata::{DirectoryVersion, DirectoryVersionReq};
use rusqlite::Connection;
use std::{fmt, sync::Arc};

/// The initial state: a matcher and name has been provided, but a match still needs to
/// be performed.
#[derive(Debug)]
pub(crate) struct PackageMatcher {
    inner: Arc<PackageMatcherInner>,
}

#[derive(Debug)]
struct PackageMatcherInner {
    hasp_home: HaspHome,
    matcher: Box<dyn PackageMatcherImpl>,
    namespace: &'static str,
    name: String,
    req: DirectoryVersionReq,
    output_opts: OutputOpts,
    db_ctx: DbContext,
}

impl PackageMatcher {
    pub(crate) fn new(
        hasp_home: HaspHome,
        matcher: Box<dyn PackageMatcherImpl>,
        name: String,
        req: DirectoryVersionReq,
        output_opts: OutputOpts,
        db_ctx: DbContext,
    ) -> Self {
        let namespace = matcher.namespace();
        Self {
            inner: Arc::new(PackageMatcherInner {
                hasp_home,
                matcher,
                namespace,
                name,
                req,
                output_opts,
                db_ctx,
            }),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn best_match(&self, conn: &Connection) -> Result<Option<DirectoryRow>> {
        let all_matches: Vec<_> =
            DirectoryRow::all_matches_for(self.namespace(), self.name(), conn)?
                .into_iter()
                .filter(|row| self.req().matches(&row.package.version))
                .collect();

        self.inner
            .matcher
            .best_match(all_matches)
            .wrap_err_with(|| format!("failed to find best match for {}", self.to_friendly()))
    }

    pub(crate) fn best_match_for_version(
        &self,
        version: &DirectoryVersion,
        conn: &Connection,
    ) -> Result<Option<DirectoryRow>> {
        let all_matches: Vec<_> =
            DirectoryRow::all_matches_for_version(self.namespace(), self.name(), version, conn)?
                .into_iter()
                .collect();

        self.inner
            .matcher
            .best_match(all_matches)
            .wrap_err_with(|| {
                format!(
                    "failed to find best match for {}:{} (version {})",
                    self.namespace(),
                    self.name(),
                    version
                )
            })
    }

    pub(crate) fn best_installed_match(&self, conn: &Connection) -> Result<Option<InstalledRow>> {
        let all_matches: Vec<_> =
            InstalledRow::all_matches_for(self.namespace(), self.name(), conn)?
                .into_iter()
                .filter(|row| self.req().matches(&row.directory_row.package.version))
                .collect();

        self.inner
            .matcher
            .best_installed_match(all_matches)
            .wrap_err_with(|| {
                format!(
                    "failed to find best installed match for {}",
                    self.to_friendly()
                )
            })
    }

    #[inline]
    pub(crate) fn hasp_home(&self) -> &HaspHome {
        &self.inner.hasp_home
    }

    #[inline]
    pub(crate) fn namespace(&self) -> &'static str {
        self.inner.namespace
    }

    #[inline]
    pub(crate) fn name(&self) -> &str {
        &self.inner.name
    }

    #[inline]
    pub(crate) fn req(&self) -> &DirectoryVersionReq {
        &self.inner.req
    }

    #[inline]
    pub(crate) fn output_opts(&self) -> OutputOpts {
        self.inner.output_opts
    }

    #[inline]
    pub(crate) fn metadata(&self) -> serde_json::Value {
        self.inner.matcher.metadata()
    }

    #[inline]
    pub(crate) fn db_ctx(&self) -> &DbContext {
        &self.inner.db_ctx
    }

    #[inline]
    pub(crate) fn make_resolver(self) -> PackageResolver {
        let resolver = self.inner.matcher.make_resolver();
        PackageResolver::new(self, resolver)
    }

    pub(crate) fn to_friendly(&self) -> String {
        format!(
            "{}:{} (version {})",
            self.namespace(),
            self.name(),
            self.req()
        )
    }
}

/// Represents a way to match a specific package.
#[async_trait]
pub(crate) trait PackageMatcherImpl: fmt::Debug {
    fn namespace(&self) -> &'static str;

    fn best_match(&self, all_matches: Vec<DirectoryRow>) -> Result<Option<DirectoryRow>>;

    /// Get the best installed row match.
    fn best_installed_match(
        &self,
        installed_rows: Vec<InstalledRow>,
    ) -> Result<Option<InstalledRow>>;

    /// Returns additional metadata to record and log as part of the package matcher.
    fn metadata(&self) -> serde_json::Value;

    /// Creates a package resolver.
    fn make_resolver(&self) -> Box<dyn PackageResolverImpl>;
}
