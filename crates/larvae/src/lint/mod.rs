/*!
`larvae lint`.

A lint is a thing that reads a parsed file and says what looks wrong. This
module holds what every lint shares: the trait, the registry, the run, and the
suppression comments that let an author say a particular one is wrong here.

The design constraint that shaped it is the editor. A lint pass runs on every
keystroke once the language server exists, so nothing here builds an index it
could have borrowed, and the shared analysis every lint wants, what each name
refers to, is computed once per file in [`ctx`] rather than once per lint.
*/

pub mod config;
pub mod ctx;
pub mod lints;
pub mod scope;

use std::path::Path;

use anyhow::Result;

use crate::diag::{Diag, Severity};
use crate::syntax::{lexer, parser};

pub use config::{Level, LintConfig};
pub use ctx::{Finding, LintCtx};

/// One thing that can be wrong with a file
pub trait Lint: Sync {
    /// What a user writes to configure it, matching selene's spelling
    fn name(&self) -> &'static str;

    /// Whether it is on by default, and how loudly
    fn default_level(&self) -> Level;

    /// One line, shown by `larvae lint --explain`
    fn about(&self) -> &'static str;

    fn run(&self, ctx: &LintCtx<'_>, out: &mut Vec<Finding>);
}

/// Every lint, in the order their findings are reported within a line
pub fn registry() -> &'static [&'static dyn Lint] {
    lints::ALL
}

/// Look up one lint by the name a user would write
pub fn find(name: &str) -> Option<&'static dyn Lint> {
    registry().iter().copied().find(|l| l.name() == name)
}

/*
Lint one file.

Parse failures come back as a diagnostic rather than an error, because a lint
run over a tree is expected to report on every file it was given and one file
that does not compile should not end the run.
*/
pub fn lint(path: &Path, src: &str, cfg: &LintConfig) -> Result<Vec<Diag>, Diag> {
    let lexed = lexer::lex(src)
        .map_err(|e| Diag::error(path, format!("syntax error, {}", e.message)).at(src, e.offset))?;

    let chunk = parser::parse(src, &lexed.toks)
        .map_err(|e| Diag::error(path, format!("syntax error, {}", e.message)).at(src, e.offset))?;

    let ctx = LintCtx::new(src, &lexed.toks, &lexed.comments, &chunk, cfg);
    let mut findings = Vec::new();

    for lint in registry() {
        let level = cfg.level_for(lint.name(), lint.default_level());

        if level == Level::Allow {
            continue;
        }

        let before = findings.len();
        lint.run(&ctx, &mut findings);

        // a lint states what it found, the config states how loudly to say it
        for finding in &mut findings[before..] {
            finding.level = level;
        }
    }

    findings.retain(|f| !ctx.suppressed(f));
    findings.sort_by_key(|f| (f.span.0, f.lint));

    Ok(findings
        .into_iter()
        .map(|f| f.into_diag(path, src))
        .collect())
}

impl Finding {
    fn into_diag(self, path: &Path, src: &str) -> Diag {
        let severity = match self.level {
            Level::Deny => Severity::Error,

            _ => Severity::Warning,
        };

        let diag = Diag {
            severity,
            file: path.to_owned(),
            line_col: None,
            message: format!("{} ({})", self.message, self.lint),
            help: self.help,
        };

        diag.at(src, self.span.0 as usize)
    }
}
