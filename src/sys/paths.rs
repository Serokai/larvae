//! The `~/.coldluau` home layout used by the `self` commands

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// `~/.coldluau`, where `self install` puts things
pub fn coldluau_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine your home directory")?
        .join(".coldluau"))
}

/// `~/.coldluau/bin`, on PATH after `self install`
pub fn bin_dir() -> Result<PathBuf> {
    Ok(coldluau_dir()?.join("bin"))
}

/// The installed binary path, `~/.coldluau/bin/coldluau[.exe]`
pub fn installed_exe() -> Result<PathBuf> {
    Ok(bin_dir()?.join(format!("coldluau{}", std::env::consts::EXE_SUFFIX)))
}

/// Same file check via canonicalize, false when either path is missing
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
