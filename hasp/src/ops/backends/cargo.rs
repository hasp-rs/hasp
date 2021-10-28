// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cargo package fetcher and installer.

use crate::{
    cargo_cli::CargoCli,
    models::directory::{DirectoryRow, InstalledRow},
    ops::{
        PackageFetcherImpl, PackageInstallerImpl, PackageMatcherImpl, PackageResolverImpl,
        TempInstalledFile, TempInstalledPackage,
    },
    output::OutputOpts,
};
use async_trait::async_trait;
use camino::{Utf8Path, Utf8PathBuf};
use cargo_metadata::Message;
use color_eyre::{
    eyre::{bail, eyre, WrapErr},
    Result,
};
use colored::Colorize;
use crates_index::{Index, IndexConfig};
use flate2::read::GzDecoder;
use hasp_metadata::{CargoDirectory, DirectoryVersion, DirectoryVersionReq};
use once_cell::sync::OnceCell;
use semver::Version;
use serde_json::Value;
use std::{collections::BTreeMap, fs, hash::Hasher, io::BufReader};
use tar::Archive;
use twox_hash::XxHash64;

#[derive(Debug)]
pub(crate) struct CargoMatcher {
    metadata: CargoDirectory,
    // TODO: features, git, registry etc
}

impl CargoMatcher {
    pub(crate) fn new(metadata: CargoDirectory) -> Self {
        Self { metadata }
    }
}

#[async_trait]
impl PackageMatcherImpl for CargoMatcher {
    #[inline]
    fn namespace(&self) -> &'static str {
        "cargo"
    }

    fn best_match(&self, rows: Vec<DirectoryRow>) -> Result<Option<DirectoryRow>> {
        // TODO: actually match on features etc
        Ok(rows.into_iter().next())
    }

    fn best_installed_match(
        &self,
        installed_rows: Vec<InstalledRow>,
    ) -> Result<Option<InstalledRow>> {
        // TODO: actually match on features etc
        Ok(installed_rows.into_iter().next())
    }

    fn metadata(&self) -> Value {
        serde_json::to_value(&self.metadata).unwrap_or(Value::Null)
    }

    fn make_resolver(&self) -> Box<dyn PackageResolverImpl> {
        Box::new(CargoResolver {
            metadata: self.metadata.clone(),
        })
    }
}

#[derive(Debug)]
struct CargoResolver {
    metadata: CargoDirectory,
}

#[async_trait]
impl PackageResolverImpl for CargoResolver {
    async fn resolve(
        &self,
        name: String,
        req: DirectoryVersionReq,
        output_opts: OutputOpts,
    ) -> Result<Box<dyn PackageFetcherImpl>> {
        let req = req
            .as_semver()
            .ok_or_else(|| eyre!("failed to parse requirement {} as semver", req.as_str()))?;

        // TODO: make it configurable, use crates.io API directly

        let (config, crate_) = {
            let mut index = Index::new_cargo_default()?;
            fetch_crates_io(&mut index)?;
            let config = index
                .index_config()
                .wrap_err("failed to get crates.io index config")?;

            let crate_ = index
                .crate_(&name)
                .ok_or_else(|| eyre!("crate '{}' not found on crates.io", name))?;
            (config, crate_)
        };

        // Look through all the versions and find the highest one that matches.
        let matching_versions: BTreeMap<Version, &crates_index::Version> = crate_
            .versions()
            .iter()
            .filter_map(|crate_info| {
                // Skip yanked versions.
                if crate_info.is_yanked() {
                    return None;
                }

                let version = match crate_info.version().parse::<Version>() {
                    Ok(version) => version,
                    Err(_) => {
                        // TODO: what to do about versions that don't parse?
                        return None;
                    }
                };

                req.matches(&version).then(|| (version, crate_info))
            })
            .collect();

        // This is the version that matches.
        let (version, crate_info) = match matching_versions.into_iter().next_back() {
            Some(x) => x,
            None => bail!("no matching version found for crate {}, req {}", name, req,),
        };

        Ok(Box::new(CargoFetcher {
            name,
            version,
            config,
            crate_info: crate_info.clone(),
            metadata: self.metadata.clone(),
            output_opts,
        }))
    }
}

#[derive(Debug)]
struct CargoFetcher {
    name: String,
    version: Version,
    config: IndexConfig,
    crate_info: crates_index::Version,
    metadata: CargoDirectory,
    output_opts: OutputOpts,
}

#[async_trait]
impl PackageFetcherImpl for CargoFetcher {
    fn version(&self) -> DirectoryVersion {
        DirectoryVersion::Semantic(self.version.clone())
    }

    async fn fetch(&self, fetch_dir: &Utf8Path) -> Result<Box<dyn PackageInstallerImpl>> {
        // Fetch this version.
        let url = self
            .crate_info
            .download_url(&self.config)
            .ok_or_else(|| eyre!("failed to create download URL"))?;
        let download_path = fetch_dir.join(format!("{}-{}.crate", self.name, self.version));

        fetch_url(&url, &download_path)
            .await
            .wrap_err_with(|| format!("failed to download {} to {}", url, download_path))?;

        // Extract the crate. (Can this be anything other than tar.gz?)
        let tar_gz = fs::File::open(&download_path)
            .wrap_err_with(|| format!("failed to open {}", download_path))?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive
            .unpack(fetch_dir)
            .wrap_err_with(|| format!("failed to extract {} as .tar.gz", download_path))?;

        let extracted_dir = fetch_dir.join(format!("{}-{}", self.name, self.version));
        Ok(Box::new(CargoInstaller {
            name: self.name.clone(),
            version: self.version.clone(),
            extracted_dir,
            metadata: self.metadata.clone(),
            output_opts: self.output_opts,
        }))
    }
}

#[derive(Debug)]
struct CargoInstaller {
    name: String,
    version: Version,
    extracted_dir: Utf8PathBuf,
    metadata: CargoDirectory,
    output_opts: OutputOpts,
    // TODO: --locked etc?
}

#[async_trait]
impl PackageInstallerImpl for CargoInstaller {
    fn installing_metadata(&self) -> Value {
        // TODO: maybe other kinds of information here, like resolved features etc
        Value::Null
    }

    fn add_to_hasher(&self, hasher: &mut XxHash64) {
        hasher.write_u8(self.metadata.default_features as u8);
    }

    async fn install(&self) -> Result<TempInstalledPackage> {
        // TODO: fetch binaries if already available
        let mut cargo_cli = CargoCli::new("build", self.output_opts);

        // TODO: features etc
        if !self.metadata.default_features {
            cargo_cli.add_arg("--no-default-features");
        }

        tracing::debug!(
            target: "hasp::output::working::building",
            "Building with cargo in {}", self.extracted_dir,
        );

        // Build the artifacts.
        let reader = cargo_cli
            .add_args(["--release", "--message-format", "json-render-diagnostics"])
            .to_expression()
            .dir(&self.extracted_dir)
            .unchecked()
            .reader()
            .wrap_err("failed to start build process")?;
        let messages = Message::parse_stream(BufReader::new(reader));

        let mut installed_files = BTreeMap::new();

        for message in messages {
            let message = message.wrap_err("failed to parse Cargo message")?;
            if let Message::CompilerArtifact(artifact) = message {
                if let Some(temp_path) = artifact.executable {
                    let file_name = temp_path.file_name().expect("file name should exist");
                    // TODO: attach metadata?
                    installed_files.insert(
                        file_name.to_owned(),
                        TempInstalledFile {
                            temp_path,
                            metadata: serde_json::Value::Null,
                            is_binary: true,
                        },
                    );
                }
            }
        }

        if installed_files.is_empty() {
            bail!("crate does not have any binaries");
        }

        // Also attach the Cargo.lock file.
        installed_files.insert(
            "Cargo.lock".to_owned(),
            TempInstalledFile {
                temp_path: self.extracted_dir.join("Cargo.lock"),
                metadata: serde_json::Value::Null,
                is_binary: false,
            },
        );

        let ret = TempInstalledPackage {
            installed_files,
            // TODO: any installed metadata to store separately from installing metadata?
            metadata: self.installing_metadata(),
        };
        Ok(ret)
    }
}

async fn fetch_url(url: &str, download_path: &Utf8Path) -> Result<()> {
    tracing::debug!(
        target: "hasp::output::working::downloading",
        "Downloading {} to {}", url.bold(), download_path.as_str().bold(),
    );
    let resp = reqwest::get(url).await?;
    let bytes = resp.bytes().await?;

    tracing::trace!(
        target: "hasp::output::working::writing_bytes",
        "Writing {} bytes to {}", bytes.len(), download_path);
    std::fs::write(download_path, &bytes)?;
    tracing::debug!(
        target: "hasp::output::downloaded",
        "Downloaded {} to {}", url, download_path,
    );

    Ok(())
}

// Fetch the crates.io index, once per process invocation.
fn fetch_crates_io(index: &mut Index) -> Result<()> {
    static FETCH_DONE: OnceCell<()> = OnceCell::new();
    FETCH_DONE.get_or_try_init(|| {
        tracing::info!(
            target: "hasp::output::working::updating_index",
            "Updating crates.io index",
        );
        index
            .update()
            .wrap_err("failed to retrieve crates.io index")
    })?;
    Ok(())
}
