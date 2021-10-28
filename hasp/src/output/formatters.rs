// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Convenience formatters for hasp data.

#![allow(dead_code)]

use colored::Colorize;
use hasp_metadata::DirectoryVersion;
use semver::Version;
use std::fmt;

pub(crate) struct NameVersionDisplay<'a> {
    name: &'a str,
    version: &'a dyn fmt::Display,
}

impl<'a> NameVersionDisplay<'a> {
    pub(crate) fn dir_version(name: &'a str, version: &'a DirectoryVersion) -> Self {
        Self {
            name,
            version: version.short_display(),
        }
    }

    pub(crate) fn semver(name: &'a str, version: &'a Version) -> Self {
        Self { name, version }
    }
}

impl<'a> fmt::Display for NameVersionDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} v{}", self.name.magenta(), self.version)
    }
}
