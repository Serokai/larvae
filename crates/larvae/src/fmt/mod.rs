/*!
`larvae fmt`, the formatter.

Four pieces, in the order a file moves through them. [`trivia`] finds the
comments the parser does not keep, [`emit`] turns the tree into a layout
document, [`doc`] decides which of that document's groups break, and [`config`]
says how wide and with what.

The property that matters is idempotence: formatting formatted output must
change nothing. It is checked as a test rather than assumed, because a formatter
that oscillates between two outputs turns every save into a diff.
*/

pub mod config;
pub mod doc;
pub mod emit;
pub mod trivia;

use anyhow::{Context, Result};

pub use config::FmtConfig;

use crate::syntax::{lexer, parser};

/// Format one file's source
pub fn format(src: &str, cfg: &FmtConfig) -> Result<String> {
    let lexed = lexer::lex(src).map_err(|e| {
        anyhow::anyhow!("syntax error at byte {}, {}", e.offset, e.message)
    })?;

    let chunk = parser::parse(src, &lexed.toks)
        .map_err(|e| anyhow::anyhow!("{}", e.message))
        .context("cannot format a file that does not parse")?;

    let trivia = trivia::Trivia::new(src, &lexed.comments);
    let emitter = emit::Emitter::new(src, &lexed.toks, &trivia, cfg);
    let document = emitter.chunk(&chunk);

    Ok(doc::render(&document, cfg.style()))
}
