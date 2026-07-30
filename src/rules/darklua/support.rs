/*!
Shared helpers for the parity rules

Small predicates the rules lean on, plus the edit shapes that show up in
more than one place. Everything here is deliberately conservative, a
predicate says yes only when the answer is provable from the tree alone
*/

use crate::rules::engine::{Edit, RuleCtx, Visit, walk_expr};
use crate::syntax::ast::*;
use crate::syntax::lexer::TokKind;

/// Luau's reserved words, a field access can never use one of these
pub fn is_reserved(word: &str) -> bool {
    matches!(
        word,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}

/// A name that can be written after a dot
pub fn is_ident(s: &str) -> bool {
    if s.is_empty() || is_reserved(s) {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/*
Side effect probe, a call is the only thing in the tree that can obviously
run user code, index and arithmetic can fire metamethods too but darklua
treats those as pure and matching it keeps ported configs predictable
*/
pub fn has_call(e: &Expr) -> bool {
    struct Probe {
        found: bool,
    }
    impl Visit for Probe {
        fn expr(&mut self, e: &Expr) {
            if matches!(e, Expr::Call { .. }) {
                self.found = true;
            }
        }
    }
    let mut p = Probe { found: false };
    walk_expr(e, &mut p);
    p.found
}

/// True when the expression needs no parentheses as an operand
pub fn is_atomic(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Nil(_)
            | Expr::True(_)
            | Expr::False(_)
            | Expr::Vararg(_)
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::InterpString(_)
            | Expr::Name(_)
            | Expr::Table { .. }
            | Expr::Paren { .. }
            | Expr::Index { .. }
            | Expr::Call { .. }
            | Expr::Function { .. }
    )
}

/*
Safe to write twice, compound assignment has to re-emit its target so the
target must not run anything, a name or a path of names qualifies and a
computed key does too as long as the key itself calls nothing
*/
pub fn is_reemittable(e: &Expr) -> bool {
    match e {
        Expr::Name(_) => true,
        Expr::Paren { inner, .. } => is_reemittable(inner),
        Expr::Index { object, key, .. } => {
            is_reemittable(object)
                && match key {
                    IndexKey::Field(_) => true,
                    IndexKey::Computed(k) => !has_call(k),
                }
        }
        _ => false,
    }
}

/// Values that Lua never treats as false, the guard remove_if_expression needs
pub fn is_never_falsy(e: &Expr) -> bool {
    match e {
        Expr::Number(_)
        | Expr::String(_)
        | Expr::InterpString(_)
        | Expr::Table { .. }
        | Expr::True(_)
        | Expr::Function { .. } => true,
        Expr::Paren { inner, .. } => is_never_falsy(inner),
        _ => false,
    }
}

// --- edit shapes ---------------------------------------------------------

/// Zero width insert at a byte offset
pub fn insert(at: u32, text: &str, edits: &mut Vec<Edit>) {
    edits.push((at, at, text.to_string()));
}

/// Byte range of one token
pub fn tok_bytes(ctx: &RuleCtx, index: u32) -> (u32, u32) {
    let t = &ctx.toks[index as usize];
    (t.start, t.end)
}

/*
Replace a byte range and pad the newline shortfall, generated text carries
whatever lines it needs and the rest are appended so retain-lines output
never drifts, refuses when the text would add lines
*/
pub fn replace_keep_lines(
    ctx: &RuleCtx,
    from: u32,
    to: u32,
    text: &str,
    edits: &mut Vec<Edit>,
) -> bool {
    let had = count_newlines(&ctx.src[from as usize..to as usize]);
    let now = count_newlines(text);
    if now > had {
        return false;
    }
    let mut out = String::with_capacity(text.len() + (had - now));
    out.push_str(text);
    for _ in 0..had - now {
        out.push('\n');
    }
    edits.push((from, to, out));
    true
}

pub fn count_newlines(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'\n').count()
}

/// True when a comment starts inside this byte range
pub fn has_comment_in(ctx: &RuleCtx, from: u32, to: u32) -> bool {
    ctx.comments.iter().any(|&(s, _)| s >= from && s < to)
}

/// Source text of an expression, wrapped in parens unless it is atomic
pub fn operand_text(ctx: &RuleCtx, e: &Expr) -> String {
    let text = ctx.text(e.span());
    if is_atomic(e) {
        text.to_string()
    } else {
        format!("({text})")
    }
}

/*
The inner content of a string token when it is a plain literal, None for
anything the rules should not reason about, escapes included, so a caller
can trust the bytes it gets back
*/
pub fn plain_string_value<'a>(ctx: &RuleCtx<'a>, span: TokSpan) -> Option<&'a str> {
    let tok = ctx.toks.get(span.start as usize)?;
    let TokKind::Str {
        inner_start,
        inner_end,
    } = tok.kind
    else {
        return None;
    };
    let inner = &ctx.src[inner_start as usize..inner_end as usize];
    if inner.contains('\\') {
        None
    } else {
        Some(inner)
    }
}

/// Token index of the `(` that opens a function body's parameter list
pub fn params_lparen(ctx: &RuleCtx, body: &FunctionBody) -> Option<u32> {
    let from = match body.generics {
        Some(g) => g.end,
        None => body.span.start,
    };
    (ctx.toks.get(from as usize)?.kind == TokKind::LParen).then_some(from)
}

/// Statements that introduce a binding, unwrapping a block past one of these
/// would leak it into the enclosing scope
pub fn declares_local(b: &Block) -> bool {
    b.stmts.iter().any(|s| {
        matches!(
            s,
            Stmt::Local(_) | Stmt::LocalFunction(_) | Stmt::TypeAlias(_)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_rules_match_luau() {
        assert!(is_ident("field"));
        assert!(is_ident("_x9"));
        assert!(is_ident("type"));
        assert!(!is_ident("end"));
        assert!(!is_ident("9a"));
        assert!(!is_ident(""));
        assert!(!is_ident("a-b"));
        assert!(!is_ident("a b"));
    }
}
