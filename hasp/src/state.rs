// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    crate_info::CrateInfo,
    database::{ConnectionCreator, DbContext},
    events::EventLogger,
    install_root::InstallRoot,
};
use camino::Utf8PathBuf;

use color_eyre::{
    eyre::{bail, WrapErr},
    Result,
};
use home::home_dir;

use std::{convert::TryInto, env, fs, path::PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct HaspState {
    home: Utf8PathBuf,
    ctx: DbContext,
}

impl HaspState {
    pub(crate) fn load_or_init() -> Result<Self> {
        let hasp_home = hasp_home()?;
        Self::load_or_init_at(hasp_home)
    }

    pub(crate) fn load_or_init_at(hasp_dir: impl Into<Utf8PathBuf>) -> Result<Self> {
        let hasp_dir = hasp_dir.into();
        fs::create_dir_all(&hasp_dir)
            .wrap_err_with(|| format!("creating hasp home at {} failed", hasp_dir))?;

        let creator = ConnectionCreator::new(&hasp_dir);
        let event_logger = EventLogger::new(&creator)?;

        // Run an initial create to initialize everything.
        creator
            .initialize(&event_logger)
            .wrap_err_with(|| format!("initializing database at {} failed", hasp_dir))?;
        Ok(Self {
            home: hasp_dir,
            ctx: DbContext {
                creator,
                event_logger,
            },
        })
    }

    pub(crate) fn install_root(&self, info: CrateInfo) -> Result<InstallRoot> {
        InstallRoot::new(info, &self.home, self.ctx.clone())
    }
}

pub(crate) fn hasp_home() -> Result<Utf8PathBuf> {
    match env::var_os("HASP_HOME") {
        Some(hasp_home) => {
            let hasp_home: Utf8PathBuf = PathBuf::from(hasp_home)
                .try_into()
                .wrap_err("HASP_HOME env var is not valid UTF-8")?;
            if hasp_home.is_relative() {
                bail!("HASP_HOME {} must be absolute", hasp_home);
            }
            Ok(hasp_home)
        },
        None => match home_dir() {
            Some(dir) => dir
                .join(".hasp")
                .try_into()
                .wrap_err("home dir is not valid UTF-8"),
            None => bail!("user home directory could not be determined (use HASP_HOME to set an explicit directory for hasp)")
        },
    }
}
