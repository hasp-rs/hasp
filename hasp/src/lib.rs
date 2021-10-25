// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    fetch_install::InstallStatus, helpers::split_version, output::OutputOpts, state::HaspState,
};
use color_eyre::{owo_colors::OwoColorize, Result};
use hasp_metadata::CargoDirectory;
use structopt::StructOpt;

mod cargo_cli;
mod database;
mod events;
mod fetch_install;
mod helpers;
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
    pub async fn exec(self) -> Result<i32> {
        self.global_opts.output.init_logger();
        self.command.exec(&self.global_opts).await
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
    async fn exec(self, global_opts: &GlobalOpts) -> Result<i32> {
        match self {
            Command::Install { crates } => {
                let state = HaspState::load_or_init()?;

                let mut already_installed = vec![];

                for spec in crates {
                    let (name, version_req) = split_version(&spec)?;
                    let status = state
                        .cargo_install(
                            name,
                            version_req.into(),
                            CargoDirectory {
                                default_features: true,
                            },
                            global_opts.output,
                        )
                        .await?;
                    match status {
                        InstallStatus::Success => {}
                        InstallStatus::Failure(err) => {
                            log::error!("{} failed to install: {:#}", spec.bold(), err);
                            return Ok(2);
                        }
                        InstallStatus::AlreadyInstalled => {
                            already_installed.push(spec);
                        }
                    }
                }

                if !already_installed.is_empty() {
                    let mut s = String::with_capacity(512);
                    for spec in &already_installed {
                        s.push_str("  * ");
                        s.push_str(&format!("{}", spec.bold()));
                        s.push('\n');
                    }
                    log::info!("these crates were already installed:\n{}", s);
                    return Ok(1);
                }

                Ok(0)
            }
        }
    }
}
