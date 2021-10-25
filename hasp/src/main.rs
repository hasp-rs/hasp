// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use color_eyre::Result;
use hasp::App;
use structopt::StructOpt;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let app = App::from_args();
    match app.exec().await {
        Ok(code) => std::process::exit(code),
        Err(err) => Err(err),
    }
}
