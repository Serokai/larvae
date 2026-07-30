//! `coldluau process`, rewrite requires into the output directory

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

use crate::config::Config;
use crate::pipeline::{self, Outcome};
use crate::ui;

pub fn run(
    root: &Path,
    config: Option<PathBuf>,
    profile: Option<String>,
    watch: bool,
) -> Result<ExitCode> {
    if profile.is_some() {
        bail!("--profile is not implemented yet (lands in M2)");
    }
    let config_path = config.clone();
    let config = load_config(root, config)?;
    if watch {
        return crate::commands::watch::run(root, &config, config_path);
    }
    let outcome = pipeline::run(root, &config, true)?;
    report(&outcome, true)
}

pub(crate) fn load_config(root: &Path, explicit: Option<PathBuf>) -> Result<Config> {
    match explicit {
        Some(path) => Config::load(&root.join(path)),
        None => Config::load_or_default(root),
    }
}

pub(crate) fn report(outcome: &Outcome, wrote: bool) -> Result<ExitCode> {
    let color = ui::want_color_stderr();
    for d in &outcome.diags {
        eprintln!("{}", d.render(color));
    }
    let s = &outcome.stats;
    let verb = if wrote { "processed" } else { "checked" };
    let mut extra = String::new();
    if wrote && s.files_copied > 0 {
        extra.push_str(&format!(", {} copied", s.files_copied));
    }
    if s.files_cached > 0 {
        extra.push_str(&format!(", {} unchanged", s.files_cached));
    }
    if s.files_pruned > 0 {
        extra.push_str(&format!(", {} stale removed", s.files_pruned));
    }
    eprintln!(
        "{} {} file(s): {} require(s) rewritten, {} dynamic require(s) left untouched{extra}",
        ui::accent(verb, color),
        s.files_processed,
        s.requires_rewritten,
        s.requires_dynamic,
    );
    if let Some(bp) = &outcome.build_project {
        eprintln!(
            "derived build project: {}",
            ui::accent(&crate::ui::rel(bp), color)
        );
    }
    Ok(if outcome.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
