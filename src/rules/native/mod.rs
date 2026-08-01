/*!
coldluau's own rules

Rules with no darklua equivalent, they lean on what only coldluau knows,
resolved require forms and the datamodel path of every file. Same contract
as the parity rules, walk the tree, push byte edits, keep newline counts
when deleting multiline spans

Every rule here is conservative by construction, when safety cannot be read
off the tree the instance is skipped silently, a wrong rewrite in a live
game costs more than a missed transform
*/

use std::path::Path;

use crate::config::RulesConfig;
use crate::diag::Diag;
use crate::rules::engine::{self, Edit, RuleCtx, Visit};
use crate::syntax::ast::{Expr, IndexKey, TokSpan};

mod dedupe_requires;
mod freeze_module;
mod inject_module_path;
mod remove_calls;
mod use_get_service;

/// True when any rule in this module is enabled, gates the parse
pub fn wants(cfg: &RulesConfig) -> bool {
    cfg.remove_calls.is_some()
        || cfg.use_get_service
        || cfg.dedupe_requires
        || cfg.inject_module_path.is_some()
        || cfg.freeze_module
}

/// Run every enabled rule, push edits and diagnostics
pub fn apply(
    cfg: &RulesConfig,
    ctx: &RuleCtx,
    edits: &mut Vec<Edit>,
    diags: &mut Vec<Diag>,
    path: &Path,
) {
    if let Some(calls) = &cfg.remove_calls {
        remove_calls::apply(calls, ctx, edits);
    }

    if cfg.use_get_service {
        use_get_service::apply(ctx, edits);
    }

    if cfg.dedupe_requires {
        dedupe_requires::apply(ctx, edits);
    }

    if let Some(name) = &cfg.inject_module_path {
        inject_module_path::apply(name, ctx, edits, diags, path);
    }

    if cfg.freeze_module {
        freeze_module::apply(ctx, edits);
    }
}

// --- shared helpers ----------------------------------------------------------

/// Text of a span that is exactly one token, ex: a binding name
fn name_text<'src>(ctx: &RuleCtx<'src>, span: TokSpan) -> &'src str {
    ctx.tok_text(span.start)
}

/// Dotted name path of a callee or receiver, ex: `debug.profilebegin`
fn dotted_path(ctx: &RuleCtx, expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(s) => Some(name_text(ctx, *s).to_string()),
        Expr::Index {
            object,
            key: IndexKey::Field(field),
            ..
        } => Some(format!(
            "{}.{}",
            dotted_path(ctx, object)?,
            name_text(ctx, *field)
        )),
        _ => None,
    }
}

/// True when the expression calls something anywhere inside it
fn contains_call(expr: &Expr) -> bool {
    struct Finder(bool);
    impl Visit for Finder {
        fn expr(&mut self, e: &Expr) {
            if matches!(e, Expr::Call { .. }) {
                self.0 = true;
            }
        }
    }

    let mut finder = Finder(false);
    engine::walk_expr(expr, &mut finder);

    finder.0
}

/// Newlines a byte range spans, a replacement must keep the same count
fn newlines_in(ctx: &RuleCtx, from: u32, to: u32) -> usize {
    ctx.src[from as usize..to as usize]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
}

/// Start of the line `at` sits on when only blanks come before it
fn blank_line_start(ctx: &RuleCtx, at: u32) -> u32 {
    let bytes = ctx.src.as_bytes();
    let mut i = at as usize;

    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }

    i as u32
}

/// Valid Luau identifier, the config checks names before they reach the output
pub fn is_ident(name: &str) -> bool {
    const KEYWORDS: [&str; 21] = [
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
        "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ];
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&name)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::syntax::scan::RequireSite;
    use crate::syntax::{lexer, parser, scan};

    /// Run the coldluau rules over a source and return the spliced output
    pub fn run(rules: &str, src: &str) -> String {
        run_full(rules, src, None, &mut Vec::new())
    }

    /// Same, with a datamodel path for this file and the diagnostics it produced
    pub fn run_full(rules: &str, src: &str, dm: Option<&str>, diags: &mut Vec<Diag>) -> String {
        let cfg: RulesConfig = toml::from_str(rules).expect("rule config");
        let lexed = lexer::lex(src).expect("lex");
        let chunk = parser::parse(src, &lexed.toks).expect("parse");

        // the unit tests stand in for the rewriter, every site keeps its own spec
        let forms: Vec<(RequireSite, String)> = scan::scan(src, &lexed.toks)
            .sites
            .iter()
            .map(|s| {
                (
                    *s,
                    src[s.inner_start as usize..s.inner_end as usize].to_string(),
                )
            })
            .collect();
        let ctx = RuleCtx {
            src,
            toks: &lexed.toks,
            chunk: &chunk,
            comments: &lexed.comments,
            require_forms: &forms,
            dm_path: dm,
            quote: '"',
        };

        let mut edits = Vec::new();
        apply(&cfg, &ctx, &mut edits, diags, Path::new("test.luau"));

        splice(src, edits)
    }

    /// The splice the pipeline runs, the first of two overlapping edits wins
    pub fn splice(src: &str, mut edits: Vec<Edit>) -> String {
        edits.sort_by_key(|e| (e.0, e.1));

        let mut out = String::new();
        let mut cursor = 0usize;

        for (start, end, new) in edits {
            if (start as usize) < cursor {
                continue;
            }

            out.push_str(&src[cursor..start as usize]);
            out.push_str(&new);
            cursor = end as usize;
        }

        out.push_str(&src[cursor..]);

        out
    }
}
