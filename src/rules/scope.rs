/*!
Which names are locals and which are globals

A rule that rewrites a bare name has to know whether the source bound it
first. Substituting a define into `local DEBUG = f()` would be silent
corruption, so this walk tracks every binding a block introduces and reports
only the name references nothing bound

Lua's binding order matters and is followed here. `local x = x` reads the
outer x on the right, a `local function f` can call itself, and a for
variable only exists inside its own loop
*/

use std::collections::HashSet;

use crate::rules::engine::RuleCtx;
use crate::syntax::ast::*;

/// Token indexes of every name reference that no enclosing scope bound
pub fn globals(ctx: &RuleCtx) -> HashSet<u32> {
    let mut b = Binder {
        ctx,
        scopes: vec![Vec::new()],
        found: HashSet::new(),
    };

    b.block(&ctx.chunk.block);

    b.found
}

struct Binder<'a, 'src> {
    ctx: &'a RuleCtx<'src>,
    scopes: Vec<Vec<&'src str>>,
    found: HashSet<u32>,
}

impl<'src> Binder<'_, 'src> {
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(&name))
    }

    fn bind(&mut self, span: TokSpan) {
        let name = self.ctx.tok_text(span.start);
        self.scopes.last_mut().expect("a scope is open").push(name);
    }

    fn open(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn close(&mut self) {
        self.scopes.pop();
    }

    fn block(&mut self, b: &Block) {
        self.open();

        for s in &b.stmts {
            self.stmt(s);
        }

        self.close();
    }

    /// A function body owns its parameters, so they live in the body's scope
    fn body(&mut self, f: &FunctionBody) {
        self.open();

        for p in &f.params {
            if !p.is_vararg {
                self.bind(p.name);
            }
        }

        for s in &f.block.stmts {
            self.stmt(s);
        }

        self.close();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Empty(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::TypeAlias(_) => {}

            // values are read before the names exist, `local x = x` sees the outer x
            Stmt::Local(n) => {
                for e in &n.values {
                    self.expr(e);
                }

                for name in &n.names {
                    self.bind(name.name);
                }
            }

            // the name comes first so the function can recurse
            Stmt::LocalFunction(n) => {
                self.bind(n.name);
                self.body(&n.body);
            }

            Stmt::Function(n) => self.body(&n.body),

            Stmt::Assign(n) => {
                for e in &n.targets {
                    self.expr(e);
                }

                for e in &n.values {
                    self.expr(e);
                }
            }

            Stmt::Call(e, _) => self.expr(e),

            Stmt::Do(n) => self.block(&n.block),

            Stmt::While(n) => {
                self.expr(&n.cond);
                self.block(&n.block);
            }

            // repeat sees the loop body's locals in its until condition
            Stmt::Repeat(n) => {
                self.open();

                for s in &n.block.stmts {
                    self.stmt(s);
                }

                self.expr(&n.cond);
                self.close();
            }

            Stmt::If(n) => {
                for (cond, body) in &n.branches {
                    self.expr(cond);
                    self.block(body);
                }

                if let Some(e) = &n.else_block {
                    self.block(e);
                }
            }

            Stmt::NumericFor(n) => {
                self.expr(&n.start);
                self.expr(&n.limit);

                if let Some(step) = &n.step {
                    self.expr(step);
                }

                self.open();
                self.bind(n.var.name);

                for s in &n.block.stmts {
                    self.stmt(s);
                }

                self.close();
            }

            Stmt::GenericFor(n) => {
                for e in &n.exprs {
                    self.expr(e);
                }

                self.open();

                for v in &n.vars {
                    self.bind(v.name);
                }

                for s in &n.block.stmts {
                    self.stmt(s);
                }

                self.close();
            }

            Stmt::Return(n) => {
                for e in &n.values {
                    self.expr(e);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(span) => {
                if !self.bound(self.ctx.tok_text(span.start)) {
                    self.found.insert(span.start);
                }
            }

            Expr::Nil(_)
            | Expr::True(_)
            | Expr::False(_)
            | Expr::Vararg(_)
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::InterpString(_) => {}

            Expr::Function { body, .. } => self.body(body),

            Expr::Table { fields, .. } => {
                for f in fields {
                    match f {
                        TableField::Positional(e) => self.expr(e),

                        TableField::Named { value, .. } => self.expr(value),

                        TableField::Computed { key, value } => {
                            self.expr(key);
                            self.expr(value);
                        }
                    }
                }
            }

            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }

            Expr::Unary { operand, .. } => self.expr(operand),

            Expr::Paren { inner, .. } => self.expr(inner),

            // a field key is not a name reference, `t.print` says nothing about print
            Expr::Index { object, key, .. } => {
                self.expr(object);

                if let IndexKey::Computed(k) = key {
                    self.expr(k);
                }
            }

            Expr::Call { func, args, .. } => {
                self.expr(func);

                match args {
                    CallArgs::Paren(list) => {
                        for a in list {
                            self.expr(a);
                        }
                    }

                    CallArgs::Table(t) => self.expr(t),

                    CallArgs::Str(_) => {}
                }
            }

            Expr::IfElse {
                branches,
                else_value,
                ..
            } => {
                for (c, val) in branches {
                    self.expr(c);
                    self.expr(val);
                }

                self.expr(else_value);
            }

            Expr::TypeAssert { expr, .. } => self.expr(expr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{lexer, parser};

    /// Names the walk decided were global, in source order
    fn found(src: &str) -> Vec<String> {
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
            defines: &Default::default(),
            globals: &Default::default(),
        };

        let mut idx: Vec<u32> = globals(&ctx).into_iter().collect();
        idx.sort_unstable();

        idx.iter().map(|i| ctx.tok_text(*i).to_string()).collect()
    }

    #[test]
    fn plain_globals() {
        assert_eq!(found("return DEBUG"), ["DEBUG"]);
        assert_eq!(found("print(DEBUG)"), ["print", "DEBUG"]);
    }

    #[test]
    fn locals_shadow() {
        assert!(found("local DEBUG = 1\nreturn DEBUG").is_empty());
        assert!(found("local function f(DEBUG) return DEBUG end").is_empty());
        assert!(found("for DEBUG = 1, 2 do return DEBUG end").is_empty());
        assert!(found("for _, DEBUG in t do return DEBUG end").contains(&"t".to_string()));
    }

    #[test]
    fn a_local_sees_the_outer_name_on_its_own_right_hand_side() {
        assert_eq!(found("local x = x"), ["x"]);
    }

    #[test]
    fn shadowing_ends_with_the_block() {
        assert_eq!(found("do local DEBUG = 1 end\nreturn DEBUG"), ["DEBUG"]);
    }

    #[test]
    fn a_field_is_not_a_name_reference() {
        assert_eq!(found("return t.DEBUG"), ["t"]);
        assert_eq!(found("return { DEBUG = 1 }"), Vec::<String>::new());
    }

    #[test]
    fn repeat_sees_its_body_in_the_condition() {
        assert!(found("repeat local ok = f() until ok").contains(&"f".to_string()));
        assert!(!found("repeat local ok = 1 until ok").contains(&"ok".to_string()));
    }
}
