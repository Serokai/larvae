/*!
Rules that drop whole call statements

Both rules share a shape, a call used as a statement whose target has a
known name goes away. `preserve_arguments_side_effects` decides what happens
when an argument might do something, on by default the call stays put
*/

use super::support;
use crate::rules::engine::{Edit, RuleCtx, Visit, walk_chunk};
use crate::syntax::ast::*;

/// remove_assertions, drop `assert(...)` statements
pub fn remove_assertions(ctx: &RuleCtx, edits: &mut Vec<Edit>, preserve: bool) {
    drop_calls(
        ctx,
        edits,
        preserve,
        &|ctx, func| matches!(func, Expr::Name(s) if ctx.text(*s) == "assert"),
    );
}

/// remove_debug_profiling, drop `debug.profilebegin` and `debug.profileend`
pub fn remove_debug_profiling(ctx: &RuleCtx, edits: &mut Vec<Edit>, preserve: bool) {
    drop_calls(ctx, edits, preserve, &|ctx, func| {
        let Expr::Index {
            object,
            key: IndexKey::Field(field),
            ..
        } = func
        else {
            return false;
        };
        let Expr::Name(base) = object.as_ref() else {
            return false;
        };
        ctx.text(*base) == "debug" && matches!(ctx.text(*field), "profilebegin" | "profileend")
    });
}

type Match = dyn Fn(&RuleCtx, &Expr) -> bool;

fn drop_calls(ctx: &RuleCtx, edits: &mut Vec<Edit>, preserve: bool, matches: &Match) {
    struct V<'a, 'b> {
        ctx: &'a RuleCtx<'b>,
        edits: &'a mut Vec<Edit>,
        preserve: bool,
        matches: &'a Match,
    }
    impl Visit for V<'_, '_> {
        fn stmt(&mut self, s: &Stmt) {
            let Stmt::Call(e, span) = s else { return };
            let Expr::Call {
                func, method, args, ..
            } = e
            else {
                return;
            };
            if method.is_some() || !(self.matches)(self.ctx, func) {
                return;
            }
            if self.preserve && args_might_do_something(args) {
                return;
            }
            self.ctx.delete_keep_lines(*span, self.edits);
        }
    }
    walk_chunk(
        ctx.chunk,
        &mut V {
            ctx,
            edits,
            preserve,
            matches,
        },
    );
}

fn args_might_do_something(args: &CallArgs) -> bool {
    match args {
        CallArgs::Paren(list) => list.iter().any(support::has_call),
        CallArgs::Table(t) => support::has_call(t),
        CallArgs::Str(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_lines_kept, run};
    use super::*;

    fn assertions(ctx: &RuleCtx, edits: &mut Vec<Edit>) {
        remove_assertions(ctx, edits, true);
    }

    fn assertions_forced(ctx: &RuleCtx, edits: &mut Vec<Edit>) {
        remove_assertions(ctx, edits, false);
    }

    fn profiling(ctx: &RuleCtx, edits: &mut Vec<Edit>) {
        remove_debug_profiling(ctx, edits, true);
    }

    #[test]
    fn plain_assertions_go() {
        let src = "assert(x)\nprint(1)\n";
        let out = run(src, assertions);
        assert!(!out.contains("assert"), "{out}");
        assert!(out.contains("print(1)"), "{out}");
        assert_lines_kept(src, &out);
        assert!(!run("assert(a == b, \"boom\")\n", assertions).contains("assert"));
    }

    #[test]
    fn arguments_with_side_effects_keep_the_call() {
        let src = "assert(check())\n";
        assert_eq!(run(src, assertions), src);
        // turning the option off removes it anyway
        assert!(!run(src, assertions_forced).contains("assert"));
    }

    #[test]
    fn assertions_used_as_values_stay() {
        // the result is bound, this is not a bare statement
        let src = "local x = assert(y)\n";
        assert_eq!(run(src, assertions), src);
        let src = "obj:assert(y)\n";
        assert_eq!(run(src, assertions), src);
    }

    #[test]
    fn debug_profiling_goes() {
        let src = "debug.profilebegin(\"x\")\nwork()\ndebug.profileend()\n";
        let out = run(src, profiling);
        assert!(!out.contains("profile"), "{out}");
        assert!(out.contains("work()"), "{out}");
        assert_lines_kept(src, &out);
    }

    #[test]
    fn other_debug_calls_stay() {
        let src = "debug.traceback()\n";
        assert_eq!(run(src, profiling), src);
        let src = "profilebegin(\"x\")\n";
        assert_eq!(run(src, profiling), src);
    }
}
