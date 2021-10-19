// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    crate_info::CrateInfo,
    crate_resolve::{exact_version_req, split_version},
    install_root::{InstallAttempted, InstallRet},
    output::OutputOpts,
    state::HaspState,
};
use color_eyre::{eyre::eyre, Result};
use hasp_metadata::DirectoryVersion;
use structopt::StructOpt;

mod cargo_cli;
mod crate_info;
mod crate_resolve;
mod database;
mod events;
mod install_root;
mod installer;
mod models;
mod output;
mod state;

#[derive(Debug, StructOpt)]
pub struct App {
    #[structopt(flatten)]
    global_opts: GlobalOpts,

    #[structopt(subcommand)]
    command: Command,
}

impl App {
    pub fn exec(self) -> Result<i32> {
        self.global_opts.output.init_logger();
        self.command.exec(&self.global_opts)
    }
}

#[derive(Clone, Debug, StructOpt)]
struct GlobalOpts {
    #[structopt(long, global = true)]
    frozen: bool,
    #[structopt(long, global = true)]
    locked: bool,
    #[structopt(long, global = true)]
    offline: bool,
    #[structopt(flatten)]
    output: OutputOpts,
}

#[derive(Debug, StructOpt)]
enum Command {
    Install {
        #[structopt(visible_alias = "crate", required = true, min_values = 1)]
        crates: Vec<String>,
        // TODO: git, registry etc
        // TODO: version req
        // TODO: features/all-features/no-default-features
        // TODO: profile
    },
}

impl Command {
    fn exec(self, global_opts: &GlobalOpts) -> Result<i32> {
        match self {
            Command::Install { crates } => {
                let state = HaspState::load_or_init()?;
                for spec in crates {
                    // TODO: Currently this takes exact versions only.
                    let (name, version_req) = split_version(&spec)?;
                    let version = exact_version_req(&version_req).ok_or_else(|| {
                        eyre!("non-exact version specified for {}: {}", name, version_req)
                    })?;
                    let crate_info = CrateInfo {
                        // TODO: non-cargo namespaces
                        namespace: "cargo".into(),
                        name,
                        version: DirectoryVersion::new_semantic(version),
                        default_features: true,
                    };
                    let install_root = state.install_root(crate_info)?;
                    let install_ret = install_root.install(global_opts.output)?;
                    match install_ret {
                        InstallRet::Attempted(InstallAttempted::Success) => {}
                        InstallRet::Attempted(InstallAttempted::Failure) => return Ok(2),
                        InstallRet::AlreadyInstalled => return Ok(1),
                    }
                }
                Ok(0)
            }
        }
    }
}
