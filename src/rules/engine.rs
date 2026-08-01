/*!
Shared machinery for ast rules

A rule walks the parsed tree and pushes byte edits, the same shape the token
rules and the require rewriter already use, so everything lands in one sorted
splice at the end. Edits apply to the original source in a single pass and a
rule never sees another rule's output. Two rules reaching for the same bytes
is handled in `edits.rs`, the first one stands and the second is reported
*/

use crate::syntax::ast::*;
use crate::syntax::lexer::Tok;
use crate::syntax::scan::RequireSite;

pub use super::edits::Edit;

/// Everything a rule gets to look at
pub struct RuleCtx<'a> {
    pub src: &'a str,
    pub toks: &'a [Tok],
    pub chunk: &'a Chunk,
    /// Byte ranges of every comment in the file
    pub comments: &'a [(u32, u32)],
    /// Each require site paired with its final emitted form, so a rule can
    /// tell that two requires point at the same module without resolving
    /// anything itself
    pub require_forms: &'a [(RequireSite, String)],
    /// The file's datamodel path when a mount covers it, ex @game/ReplicatedStorage/shared/util
    pub dm_path: Option<&'a str>,
    /// Quote char the config asked for, use it in generated strings
    pub quote: char,
}

impl<'a> RuleCtx<'a> {
    /// Byte range covered by a token span, half open
    pub fn bytes(&self, span: TokSpan) -> (u32, u32) {
        if span.is_empty() {
            let at = self
                .toks
                .get(span.start as usize)
                .map(|t| t.start)
                .unwrap_or(self.src.len() as u32);
            return (at, at);
        }

        let first = &self.toks[span.start as usize];
        let last = &self.toks[span.end as usize - 1];

        (first.start, last.end)
    }

    /// Source text covered by a token span
    pub fn text(&self, span: TokSpan) -> &'a str {
        let (a, b) = self.bytes(span);

        &self.src[a as usize..b as usize]
    }

    /// Text of one token
    pub fn tok_text(&self, index: u32) -> &'a str {
        self.toks[index as usize].text(self.src)
    }

    /*
    Delete a span but keep its newlines, retain-lines output must not shift
    line numbers, a removed multiline construct leaves blank lines behind
    the same way remove_comments does
    */
    pub fn delete_keep_lines(&self, span: TokSpan, edits: &mut Vec<Edit>) {
        let (a, b) = self.bytes(span);
        self.delete_bytes_keep_lines(a, b, edits);
    }

    pub fn delete_bytes_keep_lines(&self, from: u32, to: u32, edits: &mut Vec<Edit>) {
        let newlines = self.src[from as usize..to as usize]
            .bytes()
            .filter(|&b| b == b'\n')
            .count();
        edits.push((from, to, "\n".repeat(newlines)));
    }

    /// Replace a token span with new text
    pub fn replace(&self, span: TokSpan, text: String, edits: &mut Vec<Edit>) {
        let (a, b) = self.bytes(span);
        edits.push((a, b, text));
    }
}

/*
What the walk does after a callback returns

Skip is how a rule says it already dealt with everything under this node,
ex: a rule that rewrites a whole function head does not want its own hooks
firing again on the pieces it just claimed. It is also the cheap way for a
statement level rule to stay out of expressions it will never care about
*/
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    /// Carry on into this node's children
    Next,
    /// Leave this node's children alone
    Skip,
}

/*
Depth first walk with one callback per node family, default bodies do
nothing so a rule only overrides what it cares about, blocks are visited
before their statements and statements before their expressions
*/
pub trait Visit {
    fn block(&mut self, _b: &Block) -> Flow {
        Flow::Next
    }

    fn stmt(&mut self, _s: &Stmt) -> Flow {
        Flow::Next
    }

    fn expr(&mut self, _e: &Expr) -> Flow {
        Flow::Next
    }
}

pub fn walk_chunk(chunk: &Chunk, v: &mut impl Visit) {
    walk_block(&chunk.block, v);
}

pub fn walk_block(b: &Block, v: &mut impl Visit) {
    if v.block(b) == Flow::Skip {
        return;
    }

    for s in &b.stmts {
        walk_stmt(s, v);
    }
}

pub fn walk_stmt(s: &Stmt, v: &mut impl Visit) {
    if v.stmt(s) == Flow::Skip {
        return;
    }

    match s {
        Stmt::Empty(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::TypeAlias(_) => {}
        Stmt::Local(n) => {
            for e in &n.values {
                walk_expr(e, v);
            }
        }

        Stmt::Assign(n) => {
            for e in &n.targets {
                walk_expr(e, v);
            }

            for e in &n.values {
                walk_expr(e, v);
            }
        }

        Stmt::Call(e, _) => walk_expr(e, v),

        Stmt::Do(n) => walk_block(&n.block, v),

        Stmt::While(n) => {
            walk_expr(&n.cond, v);
            walk_block(&n.block, v);
        }

        Stmt::Repeat(n) => {
            walk_block(&n.block, v);
            walk_expr(&n.cond, v);
        }

        Stmt::If(n) => {
            for (cond, body) in &n.branches {
                walk_expr(cond, v);
                walk_block(body, v);
            }

            if let Some(e) = &n.else_block {
                walk_block(e, v);
            }
        }

        Stmt::NumericFor(n) => {
            walk_expr(&n.start, v);
            walk_expr(&n.limit, v);

            if let Some(step) = &n.step {
                walk_expr(step, v);
            }

            walk_block(&n.block, v);
        }

        Stmt::GenericFor(n) => {
            for e in &n.exprs {
                walk_expr(e, v);
            }

            walk_block(&n.block, v);
        }

        Stmt::Function(n) => walk_block(&n.body.block, v),

        Stmt::LocalFunction(n) => walk_block(&n.body.block, v),

        Stmt::Return(n) => {
            for e in &n.values {
                walk_expr(e, v);
            }
        }
    }
}

pub fn walk_expr(e: &Expr, v: &mut impl Visit) {
    if v.expr(e) == Flow::Skip {
        return;
    }

    match e {
        Expr::Nil(_)
        | Expr::True(_)
        | Expr::False(_)
        | Expr::Vararg(_)
        | Expr::Number(_)
        | Expr::String(_)
        | Expr::InterpString(_)
        | Expr::Name(_) => {}
        Expr::Function { body, .. } => walk_block(&body.block, v),

        Expr::Table { fields, .. } => {
            for f in fields {
                match f {
                    TableField::Positional(e) => walk_expr(e, v),

                    TableField::Named { value, .. } => walk_expr(value, v),

                    TableField::Computed { key, value } => {
                        walk_expr(key, v);
                        walk_expr(value, v);
                    }
                }
            }
        }

        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, v);
            walk_expr(rhs, v);
        }

        Expr::Unary { operand, .. } => walk_expr(operand, v),

        Expr::Paren { inner, .. } => walk_expr(inner, v),

        Expr::Index { object, key, .. } => {
            walk_expr(object, v);

            if let IndexKey::Computed(k) = key {
                walk_expr(k, v);
            }
        }

        Expr::Call { func, args, .. } => {
            walk_expr(func, v);

            match args {
                CallArgs::Paren(list) => {
                    for a in list {
                        walk_expr(a, v);
                    }
                }

                CallArgs::Table(t) => walk_expr(t, v),
                CallArgs::Str(_) => {}
            }
        }

        Expr::IfElse {
            branches,
            else_value,
            ..
        } => {
            for (c, val) in branches {
                walk_expr(c, v);
                walk_expr(val, v);
            }

            walk_expr(else_value, v);
        }

        Expr::TypeAssert { expr, .. } => walk_expr(expr, v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{lexer, parser};

    #[test]
    fn walk_reaches_everything() {
        let src = r#"
local t = { a = 1, [k()] = f(2 + 3) }
if x then
    for i = 1, #t do
        t[i] = if i > 2 then i else -i
    end
else
    while cond() do
        obj:method("s")
        break
    end
end
return function(...) return ... end
"#;
        let lexed = lexer::lex(src).unwrap();
        let chunk = parser::parse(src, &lexed.toks).unwrap();

        struct Counter {
            stmts: usize,
            exprs: usize,
            blocks: usize,
        }

        impl Visit for Counter {
            fn block(&mut self, _b: &Block) -> Flow {
                self.blocks += 1;

                Flow::Next
            }

            fn stmt(&mut self, _s: &Stmt) -> Flow {
                self.stmts += 1;

                Flow::Next
            }

            fn expr(&mut self, _e: &Expr) -> Flow {
                self.exprs += 1;

                Flow::Next
            }
        }

        let mut c = Counter {
            stmts: 0,
            exprs: 0,
            blocks: 0,
        };

        walk_chunk(&chunk, &mut c);
        // the exact counts matter less than nothing being skipped
        assert!(c.blocks >= 5, "blocks {}", c.blocks);
        assert!(c.stmts >= 7, "stmts {}", c.stmts);
        assert!(c.exprs >= 20, "exprs {}", c.exprs);
    }

    #[test]
    fn byte_spans_line_up() {
        let src = "local x = 1 + 2\n";
        let lexed = lexer::lex(src).unwrap();
        let chunk = parser::parse(src, &lexed.toks).unwrap();

        let ctx = RuleCtx {
            src,
            toks: &lexed.toks,
            chunk: &chunk,
            comments: &lexed.comments,
            require_forms: &[],
            dm_path: None,
            quote: '"',
        };

        let Stmt::Local(local) = &chunk.block.stmts[0] else {
            panic!()
        };

        assert_eq!(ctx.text(local.span), "local x = 1 + 2");
        let Some(Expr::Binary { span, .. }) = local.values.first() else {
            panic!()
        };

        assert_eq!(ctx.text(*span), "1 + 2");
    }
}
