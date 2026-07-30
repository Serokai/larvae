//! `coldluau check`, validate all requires without writing output

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::commands::process::{load_config, report};
use crate::pipeline;

pub fn run(root: &Path, config: Option<PathBuf>) -> Result<ExitCode> {
    let config = load_config(root, config)?;
    let outcome = pipeline::run(root, &config, false)?;
    report(&outcome, false)
}
