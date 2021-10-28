// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

mod formatters;
mod subscriber;

pub(crate) use formatters::*;

use structopt::StructOpt;

#[derive(Copy, Clone, Debug, StructOpt)]
#[must_use]
pub(crate) struct OutputOpts {
    /// Suppress output
    #[structopt(
        name = "outputquiet",
        global = true,
        long = "quiet",
        short = "q",
        conflicts_with = "outputverbose"
    )]
    pub(crate) quiet: bool,
    /// Produce extra output
    #[structopt(
        name = "outputverbose",
        global = true,
        long = "verbose",
        short = "v",
        conflicts_with = "outputquiet",
        parse(from_occurrences)
    )]
    pub(crate) verbose: usize,

    /// Produce color output
    #[structopt(
        long,
        global = true,
        default_value = "auto",
        possible_values = &["auto", "always", "never"],
    )]
    pub(crate) color: Color,
}

impl OutputOpts {
    pub(crate) fn init_logger(&self) {
        self.make_subscriber();
        self.color.init_colored();
    }

    #[allow(dead_code)]
    pub(crate) fn should_colorize(&self) -> bool {
        colored::control::SHOULD_COLORIZE.should_colorize()
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
#[must_use]
pub enum Color {
    Auto,
    Always,
    Never,
}

impl Color {
    fn init_colored(self) {
        match self {
            Color::Auto => colored::control::unset_override(),
            Color::Always => colored::control::set_override(true),
            Color::Never => colored::control::set_override(false),
        }
    }

    pub(crate) fn to_arg(self) -> &'static str {
        match self {
            Color::Auto => "--color=auto",
            Color::Always => "--color=always",
            Color::Never => "--color=never",
        }
    }
}

impl std::str::FromStr for Color {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Color::Auto),
            "always" => Ok(Color::Always),
            "never" => Ok(Color::Never),
            s => Err(format!(
                "{} is not a valid option, expected `auto`, `always` or `never`",
                s
            )),
        }
    }
}
