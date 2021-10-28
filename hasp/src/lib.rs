// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    helpers::split_version,
    ops::InstallStatus,
    output::{NameVersionDisplay, OutputOpts},
    state::HaspState,
};
use color_eyre::Result;
use futures::prelude::*;
use hasp_metadata::CargoDirectory;
use structopt::StructOpt;

mod cargo_cli;
mod database;
mod events;
mod helpers;
mod home;
mod models;
mod ops;
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

        /// Continue to install packages on encountering a failure
        #[structopt(long)]
        keep_going: bool,
        // TODO: git, registry etc
        // TODO: version req
        // TODO: features/all-features/no-default-features
        // TODO: profile
    },
}

impl Command {
    async fn exec(self, global_opts: &GlobalOpts) -> Result<i32> {
        match self {
            Command::Install { crates, keep_going } => {
                let state = HaspState::load_or_init()?;

                let mut install_futures = Vec::with_capacity(crates.len());
                for spec in crates {
                    let (name, version_req) = split_version(&spec)?;
                    let install_fut = state.cargo_install(
                        name.clone(),
                        version_req.into(),
                        CargoDirectory {
                            default_features: true,
                        },
                        global_opts.output,
                    );
                    install_futures.push(
                        install_fut.map(move |status| status.map(move |status| (name, status))),
                    );
                }

                let mut already_installed = vec![];
                let mut any_failed = false;

                for (name, status) in futures::future::try_join_all(install_futures).await? {
                    match status {
                        InstallStatus::Success { version, binaries } => {
                            let binaries_str = binaries.join(", ");
                            tracing::info!(
                                target: "hasp::output::install_success",
                                "Success {} installed with binaries {}",
                                NameVersionDisplay::dir_version(&name, &version),
                                binaries_str,
                            );
                        }
                        InstallStatus::Failure { version, report } => {
                            tracing::error!(
                                target: "hasp::output::install_failed",
                                "Failed to install {}: {:#}",
                                NameVersionDisplay::dir_version(&name, &version), report,
                            );
                            any_failed = true;
                            if !keep_going {
                                return Ok(2);
                            }
                        }
                        InstallStatus::AlreadyInstalled { version } => {
                            already_installed.push((name, version));
                        }
                    }
                }

                if !already_installed.is_empty() {
                    let mut s = String::with_capacity(512);
                    let len = already_installed.len();
                    for (idx, (name, version)) in already_installed.iter().enumerate() {
                        s.push_str("* ");
                        s.push_str(
                            format!("{}", NameVersionDisplay::dir_version(name, version)).as_str(),
                        );
                        if idx < (len - 1) {
                            s.push('\n');
                        }
                    }

                    // TODO: pass in more structured metadata once Valuable is implemented
                    tracing::info!(
                        target: "hasp::output::informational::already_installed",
                        "Info the following packages are already installed:\n{}",
                        s
                    );
                }

                if any_failed {
                    Ok(2)
                } else if !already_installed.is_empty() {
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
        }
    }
}
