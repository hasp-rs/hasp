// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, WrapErr},
    Result,
};
use fs2::FileExt;
use hasp_metadata::FileHash;
use std::{fs, hash::Hasher, io, io::Read};
use tempfile::TempDir;
use twox_hash::XxHash64;

pub(super) fn hash_file(path: &Utf8Path) -> Result<FileHash> {
    let mut hasher = blake3::Hasher::new();
    let mut file = fs::File::open(path).wrap_err_with(|| format!("failed to open {}", path))?;

    let mut buf = Vec::with_capacity(64 * 1024 * 1024);
    loop {
        let read_len = file
            .read(&mut buf)
            .wrap_err_with(|| format!("failed to read from {}", path))?;
        if read_len == 0 {
            break;
        }
        hasher.update(&buf[..read_len]);
    }

    Ok(FileHash::Blake3(hasher.finalize().into()))
}

pub(super) fn hash_bytes(bytes: impl AsRef<[u8]>, hasher: &mut XxHash64) {
    let bytes = bytes.as_ref();
    // This is similar to https://doc.rust-lang.org/beta/nightly-rustc/rustc_data_structures/stable_hasher/trait.HashStable.html.
    hasher.write_u64(bytes.len() as u64);
    hasher.write(bytes);
}

#[derive(Debug)]
pub(super) struct Utf8TempDir {
    temp_dir: TempDir,
    path: Utf8PathBuf,
}

impl Utf8TempDir {
    pub(super) fn new(parent: &Utf8Path, prefix: &str, suffix: &str) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        let temp_dir = builder
            .prefix(prefix)
            .suffix(suffix)
            .tempdir_in(parent)
            .wrap_err_with(|| format!("error creating temporary directory in {}", parent))?;

        let path = Utf8Path::from_path(temp_dir.path())
            .expect("tempdir should be UTF-8")
            .to_path_buf();
        Ok(Self { temp_dir, path })
    }

    pub(super) fn path(&self) -> &Utf8Path {
        &self.path
    }
}

/// Rename a directory to another, ignoring file not found issues.
pub(super) fn rename_non_racy(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(err) => {
            if err.kind() == io::ErrorKind::NotFound {
                // Skip this -- means src doesn't exist.
                Ok(())
            } else {
                bail!("failed to rename existing directory {} to {}", src, dest)
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct UnlockedRoot<T> {
    file: fs::File,
    lock_path: Utf8PathBuf,
    pub(super) ctx: T,
}

impl<T: AsRef<Utf8Path>> UnlockedRoot<T> {
    pub(super) fn new(ctx: T) -> Result<Self> {
        let mut lock_path = ctx.as_ref().to_path_buf();
        lock_path.set_extension(LOCKFILE_EXT);
        let mut open_opts = fs::OpenOptions::new();
        // Create the file if it doesn't exist.
        let file = open_opts
            .write(true)
            .create(true)
            .open(&lock_path)
            .wrap_err_with(|| format!("failed to open install lock at {}", lock_path))?;
        Ok(Self {
            file,
            lock_path,
            ctx,
        })
    }

    #[inline]
    pub(super) fn lock_exclusive(self) -> Result<ExclusiveRoot<T>> {
        self.file
            .lock_exclusive()
            .wrap_err_with(|| format!("failed to obtain exclusive lock at {}", self.lock_path))?;
        Ok(ExclusiveRoot {
            file: self.file,
            ctx: self.ctx,
        })
    }

    #[inline]
    #[allow(dead_code)]
    pub(super) fn lock_shared(self) -> Result<SharedRoot<T>> {
        self.file
            .lock_shared()
            .wrap_err_with(|| format!("failed to obtain shared lock at {}", self.lock_path))?;
        Ok(SharedRoot {
            file: self.file,
            ctx: self.ctx,
        })
    }
}

static LOCKFILE_EXT: &str = "lock";

/// Operations that can only be performed on a root where the shared lock has been acquired.
#[derive(Debug)]
#[must_use]
pub(super) struct SharedRoot<T> {
    file: fs::File,
    pub(super) ctx: T,
}

/// Operations that can only be performed on a root where the exclusive lock has been acquired.
/// This forms a superset of the operations on the shared root.
#[derive(Debug)]
#[must_use]
pub(super) struct ExclusiveRoot<T> {
    file: fs::File,
    pub(super) ctx: T,
}
