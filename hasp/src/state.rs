// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    database::{ConnectionCreator, DbContext},
    events::EventLogger,
    home::HaspHome,
    ops::{CargoMatcher, InstallStatus, PackageMatcher},
    output::OutputOpts,
};
use camino::Utf8PathBuf;
use color_eyre::{eyre::WrapErr, Result};
use hasp_metadata::{CargoDirectory, DirectoryVersionReq};

#[derive(Clone, Debug)]
pub(crate) struct HaspState {
    home: HaspHome,
    ctx: DbContext,
}

impl HaspState {
    pub(crate) fn load_or_init() -> Result<Self> {
        let hasp_home = HaspHome::discover()?;
        Self::load_or_init_impl(hasp_home)
    }

    #[allow(dead_code)]
    pub(crate) fn load_or_init_at(home_dir: impl Into<Utf8PathBuf>) -> Result<Self> {
        let hasp_home = HaspHome::new(home_dir.into())?;
        Self::load_or_init_impl(hasp_home)
    }

    fn load_or_init_impl(home: HaspHome) -> Result<Self> {
        let creator = ConnectionCreator::new(&home.home_dir());
        let event_logger = EventLogger::new(&creator)?;

        // Run an initial create to initialize everything.
        creator
            .initialize(&event_logger)
            .wrap_err_with(|| format!("initializing database at {} failed", home.home_dir()))?;
        Ok(Self {
            home,
            ctx: DbContext {
                creator,
                event_logger,
            },
        })
    }

    pub(crate) async fn cargo_install(
        &self,
        name: impl Into<String>,
        req: DirectoryVersionReq,
        metadata: CargoDirectory,
        output_opts: OutputOpts,
    ) -> Result<InstallStatus> {
        let matcher = CargoMatcher::new(metadata);
        let matcher = PackageMatcher::new(
            self.home.clone(),
            Box::new(matcher),
            name.into(),
            req,
            output_opts,
            self.ctx.clone(),
        );

        let mut conn = self.ctx.creator.create()?;
        let txn = conn.transaction()?;

        match matcher.best_installed_match(&txn)? {
            Some(_) => {
                // TODO: force install/update?
                Ok(InstallStatus::AlreadyInstalled)
            }
            None => {
                // Perform the resolve/fetch/install operations.
                let resolver = matcher.make_resolver();
                let fetcher = resolver.make_fetcher().await?;
                let installer = fetcher.fetch().await?;
                installer.install(false).await
            }
        }
    }
}
