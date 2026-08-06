/*!
`larvae lint`.

Shaped like `larvae fmt` on purpose: the same path handling, the same default
target, the same parallel walk. Someone who has run one should not have to
learn the other.

The exit code is the part worth stating. A warning reports and exits zero, an
error exits one, so a project decides what fails CI by setting levels rather
than by choosing a different command.
*/

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::commands::fmt::collect;
use crate::config::Config;
use crate::diag::{Diag, Severity};
use crate::lint::{LintConfig, lint, registry};
use crate::ui;

pub fn run(
    root: &Path,
    paths: Vec<PathBuf>,
    stdin: bool,
    explain: Option<String>,
    config: Option<PathBuf>,
) -> Result<ExitCode> {
    if let Some(name) = explain {
        return explain_lint(&name);
    }

    let cfg = discover(root, config)?;

    if stdin {
        return from_stdin(&cfg);
    }

    let files = collect(root, &paths)?;

    if files.is_empty() {
        ui::print_error("no Luau files found");

        return Ok(ExitCode::FAILURE);
    }

    let mut diags: Vec<Diag> = files
        .par_iter()
        .flat_map(|path| match std::fs::read_to_string(path) {
            Ok(src) => match lint(path, &src, &cfg) {
                Ok(found) => found,

                Err(e) => vec![e],
            },

            Err(e) => vec![Diag::error(path, format!("cannot read, {e}"))],
        })
        .collect();

    diags.sort_by(|a, b| a.file.cmp(&b.file).then(a.line_col.cmp(&b.line_col)));

    report(&diags, files.len())
}

fn discover(root: &Path, config: Option<PathBuf>) -> Result<LintConfig> {
    let path = config.unwrap_or_else(|| root.join("larvae.toml"));

    let larvae = match path.exists() {
        true => Config::load(&path)?.lint,

        false => None,
    };

    LintConfig::discover(root, larvae.as_ref())
}

/// One file over the pipes, which is how an editor asks
fn from_stdin(cfg: &LintConfig) -> Result<ExitCode> {
    let mut src = String::new();
    std::io::stdin()
        .read_to_string(&mut src)
        .context("cannot read stdin")?;

    let path = Path::new("stdin");

    let diags = match lint(path, &src, cfg) {
        Ok(found) => found,

        Err(e) => vec![e],
    };

    report(&diags, 1)
}

/// `--explain <name>`, so a finding can be looked up without leaving the terminal
fn explain_lint(name: &str) -> Result<ExitCode> {
    if let Some(found) = crate::lint::find(name) {
        println!("{}\n  {}", found.name(), found.about());
        println!("  default: {:?}", found.default_level());

        return Ok(ExitCode::SUCCESS);
    }

    ui::print_error(&format!("no lint called {name}"));
    println!("\navailable lints:");

    // sorted for looking one up, and sized so the second column stays a column
    let mut all: Vec<_> = registry().iter().collect();
    all.sort_by_key(|l| l.name());

    let width = all.iter().map(|l| l.name().len()).max().unwrap_or(0);

    for lint in all {
        println!("  {:<width$}  {}", lint.name(), lint.about());
    }

    Ok(ExitCode::FAILURE)
}

fn files(n: usize) -> String {
    match n {
        1 => "1 file".to_string(),

        n => format!("{n} files"),
    }
}

fn report(diags: &[Diag], scanned: usize) -> Result<ExitCode> {
    let color = ui::want_color();

    for diag in diags {
        println!("{}", diag.render(color));
    }

    let errors = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();

    let warnings = diags.len() - errors;

    if diags.is_empty() {
        ui::print_success(&format!("{}, nothing to report", files(scanned)));

        return Ok(ExitCode::SUCCESS);
    }

    println!("\n{}, {errors} errors, {warnings} warnings", files(scanned));

    /*
    Only an error fails the run. A project that wants a warning to fail CI
    raises it to deny in its config, which keeps the decision in one place
    rather than splitting it between the config and the command line.
    */
    match errors {
        0 => Ok(ExitCode::SUCCESS),

        _ => Ok(ExitCode::FAILURE),
    }
}
