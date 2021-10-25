// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    database::{ConnectionCreator, DbContext},
    events::EventLogger,
    fetch_install::{CargoMatcher, InstallStatus, PackageMatcher},
    output::OutputOpts,
};
use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, WrapErr},
    Result,
};
use hasp_metadata::{CargoDirectory, DirectoryHash, DirectoryVersionReq};
use home::home_dir;
use std::{convert::TryInto, env, fs, path::PathBuf};

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

#[derive(Clone, Debug)]
pub(crate) struct HaspHome {
    home_dir: Utf8PathBuf,
    cache_dir: Utf8PathBuf,
    installs_dir: Utf8PathBuf,
}

impl HaspHome {
    pub(crate) fn new(home_dir: impl Into<Utf8PathBuf>) -> Result<Self> {
        let home_dir = home_dir.into();
        let cache_dir = home_dir.join("cache");
        let installs_dir = home_dir.join("installs");

        // The home directory will automatically be created.
        fs::create_dir_all(&cache_dir)
            .wrap_err_with(|| format!("failed to create {}", cache_dir))?;
        fs::create_dir_all(&installs_dir)
            .wrap_err_with(|| format!("failed to create {}", installs_dir))?;
        Ok(Self {
            home_dir,
            cache_dir,
            installs_dir,
        })
    }

    pub(crate) fn discover() -> Result<Self> {
        let home = match env::var_os("HASP_HOME") {
            Some(hasp_home) => {
                let hasp_home: Utf8PathBuf = PathBuf::from(hasp_home)
                    .try_into()
                    .wrap_err("HASP_HOME env var is not valid UTF-8")?;
                if hasp_home.is_relative() {
                    bail!("HASP_HOME {} must be absolute", hasp_home);
                }
                hasp_home
            },
            None => match home_dir() {
                Some(dir) => dir
                    .join(".hasp")
                    .try_into()
                    .wrap_err("home dir is not valid UTF-8")?,
                None => bail!("user home directory could not be determined (use HASP_HOME to set an explicit directory for hasp)")
            },
        };

        Self::new(home)
    }

    #[inline]
    pub(crate) fn home_dir(&self) -> &Utf8Path {
        &self.home_dir
    }

    #[inline]
    pub(crate) fn cache_dir(&self) -> &Utf8Path {
        &self.cache_dir
    }

    #[inline]
    pub(crate) fn installs_dir(&self) -> &Utf8Path {
        &self.installs_dir
    }

    pub(crate) fn make_install_path(
        &self,
        namespace: &'static str,
        name: &str,
        hash: DirectoryHash,
    ) -> Result<Utf8PathBuf> {
        let mut install_path = self.installs_dir().join(namespace);
        install_path.push(name);
        install_path.push(&format!("{}", hash));

        fs::create_dir_all(&install_path)
            .wrap_err_with(|| format!("failed to create directory at {}", install_path))?;
        Ok(install_path)
    }
}
